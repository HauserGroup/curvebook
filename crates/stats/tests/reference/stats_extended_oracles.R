# Independent R 4.5.2 (2025-10-31) black-box reference generator for
# `d1_extended_stats_oracles.rs` and tightened `levene_variance_test` checks
# in `d1_reference_oracles.rs`.
#
# Run under R 4.5.2 (2025-10-31):
#   Rscript crates/stats/tests/reference/stats_extended_oracles.R
#
# Note: Normality (D'Agostino-Pearson K^2, Anderson-Darling, Lilliefors),
# two-stage BKY FDR, and Cohen's kappa are generated separately by independent
# black-box SciPy 1.18.1 + statsmodels 0.15.0 in `scipy_statsmodels_oracles.py`.

options(digits = 16)

cat("=== 0. Existing d1_reference_oracles g_a vs g_b Brown-Forsythe Levene ===\n")
g_a <- c(23.1, 25.4, 21.8, 24.0, 26.2, 22.7)
g_b <- c(18.5, 20.1, 17.9, 21.3, 19.2, 18.8, 20.6)
levene_calc <- function(groups, use_median = TRUE) {
  g <- factor(rep(seq_along(groups), lengths(groups)))
  z <- unlist(lapply(groups, function(v) abs(v - if (use_median) median(v) else mean(v))))
  fit <- anova(lm(z ~ g))
  c(F = fit$`F value`[1], df1 = fit$Df[1], df2 = fit$Df[2], p = fit$`Pr(>F)`[1])
}
print(levene_calc(list(g_a, g_b), TRUE))
print(levene_calc(list(g_a, g_b), FALSE))

cat("\n=== 1. Two-sample KS (overlap exact, ties asymptotic, large-n asymptotic) ===\n")
ks_x1 <- c(1.2, 2.5, 3.1, 4.8, 5.2, 6.9, 7.4, 8.8)
ks_y1 <- c(2.0, 3.4, 4.1, 5.5, 6.2, 7.9, 9.1, 10.3, 11.4)
r_ks1 <- ks.test(ks_x1, ks_y1, exact = TRUE)
cat(sprintf("overlap_exact: D=%.15f p=%.15f\n", r_ks1$statistic, r_ks1$p.value))

ks_x2 <- c(1.0, 2.0, 2.0, 3.0, 4.0, 5.0, 5.0, 6.0, 7.0, 8.0)
ks_y2 <- c(2.0, 3.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0)
r_ks2 <- suppressWarnings(ks.test(ks_x2, ks_y2, exact = FALSE))
cat(sprintf("ties_asymp: D=%.15f p=%.15f\n", r_ks2$statistic, r_ks2$p.value))

ks_x3 <- seq(0.5, 40.0, length.out = 40) + sin(1:40) * 1.5
ks_y3 <- seq(6.0, 46.0, length.out = 45) + cos(1:45) * 1.5
r_ks3 <- ks.test(ks_x3, ks_y3, exact = FALSE)
cat(sprintf("large_asymp: D=%.15f p=%.15f\n", r_ks3$statistic, r_ks3$p.value))

cat("\n=== 2. Games-Howell (2D quadrature + qtukey) & Dunnett T3 (SMM quadrature) ===\n")
het <- list(
  G1 = c(10.2, 10.8, 9.9, 11.1, 10.5, 10.3),
  G2 = c(12.1, 14.8, 11.2, 15.6, 13.4, 16.2, 12.9, 14.1),
  G3 = c(18.5, 22.0, 16.4, 24.1, 19.8),
  G4 = c(25.0, 25.4, 24.8, 25.7, 25.1, 25.5, 24.9)
)
k <- length(het)
m <- k * (k - 1) / 2
ns <- sapply(het, length)
ms <- sapply(het, mean)
vs <- sapply(het, var)

# Studentized maximum modulus CDF via independent adaptive quadrature:
# P(max_{1..m} |T_l| <= t) = E_S[ (2*Phi(t*S) - 1)^m ], where S = sqrt(chi2_df / df).
smm_cdf <- function(t_abs, m_pairs, df) {
  integrand <- function(u) {
    s <- sqrt(u / df)
    exp(dchisq(u, df = df, log = TRUE) + m_pairs * log(pmax(2 * pnorm(t_abs * s) - 1, 0)))
  }
  integrate(integrand, lower = 0, upper = Inf, rel.tol = 1e-12, abs.tol = 1e-13)$value
}
smm_sf <- function(t_abs, m_pairs, df) max(0, min(1, 1 - smm_cdf(t_abs, m_pairs, df)))
smm_crit <- function(alpha, m_pairs, df) {
  uniroot(function(x) smm_sf(x, m_pairs, df) - alpha, c(0.1, 50), tol = 1e-12)$root
}

# Exact 2D Studentized range survival probability P(Q_{k,df} > q) via R integrate().
q_sf_exact <- function(q, k_means, df) {
  range_sf <- function(w) {
    sapply(w, function(wi) {
      1 - integrate(function(z) k_means * dnorm(z) * (pnorm(z + wi) - pnorm(z))^(k_means - 1),
                    -12, 12, rel.tol = 1e-12, abs.tol = 1e-15)$value
    })
  }
  integrate(function(u) {
    s <- sqrt(u / df)
    dchisq(u, df = df) * range_sf(q * s)
  }, 0, Inf, rel.tol = 1e-11, abs.tol = 1e-16)$value
}

alpha <- 0.05
for (i in 1:(k - 1)) {
  for (j in (i + 1):k) {
    wt <- t.test(het[[i]], het[[j]], var.equal = FALSE)
    diff <- ms[i] - ms[j]
    se <- abs(diff / unname(wt$statistic))
    df <- unname(wt$parameter)
    t_stat <- unname(wt$statistic)

    # Games-Howell
    se_q <- se / sqrt(2)
    gh_q <- abs(diff) / se_q
    gh_p <- q_sf_exact(gh_q, k, df)
    gh_qc <- qtukey(1 - alpha, k, df)
    gh_ci <- c(diff - gh_qc * se_q, diff + gh_qc * se_q)

    # Dunnett T3
    dt3_p <- smm_sf(abs(t_stat), m, df)
    dt3_tc <- smm_crit(alpha, m, df)
    dt3_ci <- c(diff - dt3_tc * se, diff + dt3_tc * se)

    cat(sprintf("Pair (%d,%d) diff=%.12f se=%.12f df=%.12f t=%.12f\n",
                i - 1, j - 1, diff, se, df, t_stat))
    cat(sprintf("  GH:  q=%.12f p=%.12e ci=[%.12f, %.12f]\n", gh_q, gh_p, gh_ci[1], gh_ci[2]))
    cat(sprintf("  DT3: p=%.12e ci=[%.12f, %.12f]\n", dt3_p, dt3_ci[1], dt3_ci[2]))
  }
}

cat("\n=== 3. Published worked examples (Levene, NIST Grubbs, Bland-Altman 1986) ===\n")
cat("Levene on het (median / mean):\n")
print(levene_calc(het, TRUE))
print(levene_calc(het, FALSE))

# NIST/SEMATECH e-Handbook Grubbs worked example (n = 8, certified G = 2.4687)
grubbs_nist <- c(199.31, 199.53, 200.19, 200.82, 201.92, 201.95, 202.18, 245.57)
gn <- length(grubbs_nist)
G_val <- max(abs(grubbs_nist - mean(grubbs_nist))) / sd(grubbs_nist)
t_crit_g <- qt(1 - 0.05 / (2 * gn), df = gn - 2)
G_crit <- ((gn - 1) / sqrt(gn)) * sqrt(t_crit_g^2 / (gn - 2 + t_crit_g^2))
cat(sprintf("Grubbs NIST (n=8): G=%.12f G_crit=%.12f\n", G_val, G_crit))

# Bland & Altman (Lancet 1986, Table 1, n = 17 Wright vs Mini-Wright PEFR)
ba_wright <- c(494, 395, 516, 434, 476, 557, 413, 442, 650, 433, 417, 656, 267, 478, 178, 423, 427)
ba_mini   <- c(512, 430, 520, 428, 500, 600, 364, 380, 658, 445, 432, 626, 260, 477, 259, 350, 451)
ba_diff <- ba_wright - ba_mini
ba_n <- length(ba_diff)
ba_bias <- mean(ba_diff)
ba_sd <- sd(ba_diff)
z95 <- qnorm(0.975)
t95 <- qt(0.975, df = ba_n - 1)
ba_se_bias <- ba_sd / sqrt(ba_n)
ba_se_loa <- ba_sd * sqrt(1 / ba_n + z95^2 / (2 * (ba_n - 1)))
ba_loa_lo <- ba_bias - z95 * ba_sd
ba_loa_hi <- ba_bias + z95 * ba_sd
cat(sprintf("Bland-Altman 1986: bias=%.12f sd=%.12f loa=[%.12f, %.12f]\n",
            ba_bias, ba_sd, ba_loa_lo, ba_loa_hi))
cat(sprintf("  bias_ci=[%.12f, %.12f] loa_lo_ci=[%.12f, %.12f] loa_hi_ci=[%.12f, %.12f]\n",
            ba_bias - t95 * ba_se_bias, ba_bias + t95 * ba_se_bias,
            ba_loa_lo - t95 * ba_se_loa, ba_loa_lo + t95 * ba_se_loa,
            ba_loa_hi - t95 * ba_se_loa, ba_loa_hi + t95 * ba_se_loa))

cat("\n=== 4. Noncentral-t and Noncentral-F effect-size CIs ===\n")
nct_ci <- function(t_obs, df, conf = 0.95) {
  a <- (1 - conf) / 2
  lo <- uniroot(function(ncp) pt(t_obs, df, ncp = ncp) - (1 - a), c(t_obs - 25, t_obs + 25), tol = 1e-12)$root
  hi <- uniroot(function(ncp) pt(t_obs, df, ncp = ncp) - a, c(t_obs - 25, t_obs + 25), tol = 1e-12)$root
  c(lo, hi)
}
ea <- c(12.4, 13.1, 11.8, 12.9, 13.5, 12.2, 12.7)
eb <- c(10.1, 10.8, 9.7, 11.0, 10.4, 10.2, 9.9, 10.6)
na <- length(ea); nb <- length(eb); df_ab <- na + nb - 2
sp <- sqrt(((na - 1) * var(ea) + (nb - 1) * var(eb)) / df_ab)
d_ab <- (mean(ea) - mean(eb)) / sp
t_ab <- (mean(ea) - mean(eb)) / (sp * sqrt(1 / na + 1 / nb))
ncp_ab <- nct_ci(t_ab, df_ab)
d_ci <- ncp_ab * sqrt(1 / na + 1 / nb)
J_ab <- exp(lgamma(df_ab / 2) - 0.5 * log(df_ab / 2) - lgamma((df_ab - 1) / 2))
cat(sprintf("Unpaired moderate: d=%.12f d_ci=[%.12f, %.12f] g=%.12f g_ci=[%.12f, %.12f]\n",
            d_ab, d_ci[1], d_ci[2], J_ab * d_ab, J_ab * d_ci[1], J_ab * d_ci[2]))

za <- c(10.0, 10.5, 9.5, 10.2, 9.8, 10.1)
zb <- c(10.1, 9.9, 10.4, 9.6, 10.2, 9.9)
nza <- length(za); nzb <- length(zb); df_z <- nza + nzb - 2
sp_z <- sqrt(((nza - 1) * var(za) + (nzb - 1) * var(zb)) / df_z)
d_z <- (mean(za) - mean(zb)) / sp_z
t_z <- (mean(za) - mean(zb)) / (sp_z * sqrt(1 / nza + 1 / nzb))
d_z_ci <- nct_ci(t_z, df_z) * sqrt(1 / nza + 1 / nzb)
cat(sprintf("Unpaired near-zero: d=%.12f d_ci=[%.12f, %.12f]\n", d_z, d_z_ci[1], d_z_ci[2]))

pa <- c(120, 132, 118, 140, 125, 135, 128, 144)
pb <- c(112, 124, 115, 129, 121, 126, 120, 133)
pd <- pa - pb; np <- length(pd); df_p <- np - 1
dz <- mean(pd) / sd(pd); t_p <- dz * sqrt(np)
J_p <- exp(lgamma(df_p / 2) - 0.5 * log(df_p / 2) - lgamma((df_p - 1) / 2))
gz_ci <- J_p * nct_ci(t_p, df_p) / sqrt(np)
cat(sprintf("Paired moderate: dz=%.12f gz=%.12f gz_ci=[%.12f, %.12f]\n", dz, J_p * dz, gz_ci[1], gz_ci[2]))

ncf_eta2_ci <- function(f_obs, df1, df2, conf = 0.95) {
  a <- (1 - conf) / 2
  lo_ncp <- if (pf(f_obs, df1, df2, ncp = 0) <= 1 - a) 0 else {
    uniroot(function(ncp) pf(f_obs, df1, df2, ncp = ncp) - (1 - a), c(0, 5000), tol = 1e-12)$root
  }
  hi_ncp <- if (pf(f_obs, df1, df2, ncp = 0) <= a) 0 else {
    uniroot(function(ncp) pf(f_obs, df1, df2, ncp = ncp) - a, c(0, 5000), tol = 1e-12)$root
  }
  n_tot <- df1 + df2 + 1
  c(lo_ncp / (lo_ncp + n_tot), hi_ncp / (hi_ncp + n_tot))
}
nk <- list(
  A = c(10.0, 11.4, 9.2, 10.8, 10.1),
  B = c(10.6, 12.1, 9.8, 11.3, 10.7),
  C = c(11.3, 12.7, 10.4, 11.9, 11.2),
  D = c(13.8, 15.2, 12.9, 14.6, 14.0)
)
aov_sig <- summary(aov(y ~ g, data = data.frame(y = unlist(nk), g = factor(rep(names(nk), lengths(nk))))))[[1]]
f_sig <- aov_sig$`F value`[1]; df1_s <- aov_sig$Df[1]; df2_s <- aov_sig$Df[2]
eta2_sig <- aov_sig$`Sum Sq`[1] / sum(aov_sig$`Sum Sq`)
ci_sig <- ncf_eta2_ci(f_sig, df1_s, df2_s)
cat(sprintf("ANOVA sig: F=%.12f eta2=%.12f eta2_ci=[%.12f, %.12f]\n", f_sig, eta2_sig, ci_sig[1], ci_sig[2]))

low_g <- list(A = c(10.0, 11.2, 9.4, 10.6, 9.8), B = c(10.5, 11.1, 9.7, 10.8, 10.2), C = c(9.9, 11.0, 9.6, 10.4, 10.1))
aov_low <- summary(aov(y ~ g, data = data.frame(y = unlist(low_g), g = factor(rep(names(low_g), lengths(low_g))))))[[1]]
f_low <- aov_low$`F value`[1]; df1_l <- aov_low$Df[1]; df2_l <- aov_low$Df[2]
eta2_low <- aov_low$`Sum Sq`[1] / sum(aov_low$`Sum Sq`)
ci_low <- ncf_eta2_ci(f_low, df1_l, df2_l)
cat(sprintf("ANOVA low-F: F=%.12f eta2=%.12f eta2_ci=[%.12f, %.12f]\n", f_low, eta2_low, ci_low[1], ci_low[2]))

cat("\n=== 5. Brown-Forsythe F* with Satterthwaite df ===\n")
bf_f_star <- function(groups) {
  ns <- lengths(groups)
  ms <- sapply(groups, mean)
  vs <- sapply(groups, var)
  N <- sum(ns)
  k <- length(groups)
  gm <- sum(ns * ms) / N
  ssb <- sum(ns * (ms - gm)^2)
  c_terms <- (1 - ns / N) * vs
  denom <- sum(c_terms)
  f_star <- ssb / denom
  df1 <- k - 1
  df2 <- denom^2 / sum(c_terms^2 / (ns - 1))
  p <- pf(f_star, df1, df2, lower.tail = FALSE)
  c(F_star = f_star, df1 = df1, df2 = df2, p = p)
}
het_df <- data.frame(y = unlist(het), g = factor(rep(names(het), lengths(het))))
wt_het <- oneway.test(y ~ g, data = het_df, var.equal = FALSE)
bf_het <- bf_f_star(het)
cat(sprintf("het Welch: W=%.12f df1=%.12f df2=%.12f p=%.12e\n",
            wt_het$statistic, wt_het$parameter[1], wt_het$parameter[2], wt_het$p.value))
cat(sprintf("het BF F*: F*=%.12f df1=%.12f df2=%.12f p=%.12e\n",
            bf_het["F_star"], bf_het["df1"], bf_het["df2"], bf_het["p"]))

bal <- list(
  G1 = c(10.0, 10.2, 9.8, 10.1, 9.9),
  G2 = c(12.0, 14.5, 11.2, 13.8, 12.5),
  G3 = c(15.0, 19.0, 14.2, 17.8, 16.0),
  G4 = c(20.0, 20.5, 19.7, 20.3, 19.5)
)
bal_df <- data.frame(y = unlist(bal), g = factor(rep(names(bal), lengths(bal))))
aov_bal <- summary(aov(y ~ g, data = bal_df))[[1]]
bf_bal <- bf_f_star(bal)
cat(sprintf("bal ordinary F=%.12f vs BF F*=%.12f df2=%.12f p=%.12e\n",
            aov_bal$`F value`[1], bf_bal["F_star"], bf_bal["df2"], bf_bal["p"]))

cat("\n=== 6. Omega^2 CI & Steiger (2004) N=30/36 provenance ===\n")
# Unbalanced Ctrl/Low/High/Mid from `d1_reference_oracles.rs`
u_ctrl <- c(6.2, 6.5, 6.8, 7.1, 6.4)
u_low  <- c(7.9, 8.3, 8.0, 8.6, 8.2, 8.5)
u_high <- c(9.4, 9.8, 9.1, 9.6)
u_mid  <- c(7.0, 7.3, 6.9, 7.4, 7.2)
aov_uclhm <- summary(aov(y ~ g, data = data.frame(
  y = c(u_ctrl, u_low, u_high, u_mid),
  g = factor(rep(c("Ctrl", "Low", "High", "Mid"), c(5, 6, 4, 5)))
)))[[1]]
ssb_u <- aov_uclhm$`Sum Sq`[1]; ssw_u <- aov_uclhm$`Sum Sq`[2]; sst_u <- ssb_u + ssw_u
df1_u <- aov_uclhm$Df[1]; df2_u <- aov_uclhm$Df[2]; msw_u <- ssw_u / df2_u
f_u <- aov_uclhm$`F value`[1]
eta2_u <- ssb_u / sst_u
omega2_u <- (ssb_u - df1_u * msw_u) / (sst_u + msw_u)
cohens_f_u <- sqrt(eta2_u / (1 - eta2_u))
ci_u95 <- ncf_eta2_ci(f_u, df1_u, df2_u, conf = 0.95)
fci_u95 <- sqrt(ci_u95 / (1 - ci_u95))
cat(sprintf("unbal clhm: F=%.12f eta2=%.12f omega2=%.12f f=%.12f\n", f_u, eta2_u, omega2_u, cohens_f_u))
cat(sprintf("  Steiger 95%% pop_var_ci=[%.12f, %.12f] cohens_f_ci=[%.12f, %.12f]\n",
            ci_u95[1], ci_u95[2], fci_u95[1], fci_u95[2]))

ctrl <- c(10.2, 11.4, 9.8, 10.6, 11.0)
low  <- c(14.1, 15.3, 13.8, 14.9, 14.4)
high <- c(18.2, 19.0, 17.6, 18.8, 18.4)
mid  <- c(12.0, 12.6, 11.5, 12.3, 12.1)
aov_clhm <- summary(aov(y ~ g, data = data.frame(
  y = c(ctrl, low, high, mid),
  g = factor(rep(c("Ctrl", "Low", "High", "Mid"), each = 5))
)))[[1]]
ssb_c <- aov_clhm$`Sum Sq`[1]; ssw_c <- aov_clhm$`Sum Sq`[2]; sst_c <- ssb_c + ssw_c
df1_c <- aov_clhm$Df[1]; df2_c <- aov_clhm$Df[2]; msw_c <- ssw_c / df2_c
f_c <- aov_clhm$`F value`[1]
eta2_c <- ssb_c / sst_c
omega2_c <- (ssb_c - df1_c * msw_c) / (sst_c + msw_c)
cohens_f_c <- sqrt(eta2_c / (1 - eta2_c))
ci_c95 <- ncf_eta2_ci(f_c, df1_c, df2_c, conf = 0.95)
fci_c95 <- sqrt(ci_c95 / (1 - ci_c95))
cat(sprintf("clhm: F=%.12f eta2=%.12f omega2=%.12f f=%.12f\n", f_c, eta2_c, omega2_c, cohens_f_c))
cat(sprintf("  Steiger 95%% pop_var_ci=[%.12f, %.12f] cohens_f_ci=[%.12f, %.12f]\n",
            ci_c95[1], ci_c95[2], fci_c95[1], fci_c95[2]))

ncf_steiger_for_n <- function(ss_eff, ss_err, ss_tot, df1, df2, N, conf = 0.95) {
  a <- (1 - conf) / 2
  ms_err <- ss_err / df2
  f_obs <- (ss_eff / df1) / ms_err
  lo_ncp <- if (pf(f_obs, df1, df2, ncp = 0) <= 1 - a) 0 else {
    uniroot(function(ncp) pf(f_obs, df1, df2, ncp = ncp) - (1 - a), c(0, 5000), tol = 1e-13)$root
  }
  hi_ncp <- if (pf(f_obs, df1, df2, ncp = 0) <= a) 0 else {
    uniroot(function(ncp) pf(f_obs, df1, df2, ncp = ncp) - a, c(0, 5000), tol = 1e-13)$root
  }
  p_eta2 <- ss_eff / (ss_eff + ss_err)
  omega2 <- (ss_eff - df1 * ms_err) / (ss_tot + ms_err)
  ci_p <- c(lo_ncp / (lo_ncp + N), hi_ncp / (hi_ncp + N))
  ci_f <- sqrt(ci_p / (1 - ci_p))
  list(f_obs = f_obs, p_eta2 = p_eta2, omega2 = omega2, lo_ncp = lo_ncp, hi_ncp = hi_ncp, ci_p = ci_p, ci_f = ci_f)
}
for (N_val in c(30, 36)) {
  st <- ncf_steiger_for_n(25, 60, 150, 2, 24, N_val, 0.95)
  cat(sprintf("Steiger N=%d: F=%.12f p_eta2=%.12f omega2=%.12f ncp=[%.12f, %.12f] ci_p=[%.12f, %.12f] ci_f=[%.12f, %.12f]\n",
              N_val, st$f_obs, st$p_eta2, st$omega2, st$lo_ncp, st$hi_ncp, st$ci_p[1], st$ci_p[2], st$ci_f[1], st$ci_f[2]))
}

cat("\n=== 7. mean_ci vs stats::t.test (+ sd_ci & geometric_mean_ci) ===\n")
pk <- c(2.4, 3.1, 3.8, 4.2, 5.0, 6.5, 8.2, 11.4)
signed_v <- c(-4.5, -1.2, 0.0, 2.3, 3.8, 5.1, -6.0)
for (cl in c(0.90, 0.95, 0.99)) {
  tt_pk <- t.test(pk, conf.level = cl)
  tt_g <- t.test(log(pk), conf.level = cl)
  alpha_c <- 1 - cl; df_pk <- length(pk) - 1; s_pk <- sd(pk)
  sd_ci <- c(sqrt(df_pk * s_pk^2 / qchisq(1 - alpha_c / 2, df_pk)),
             sqrt(df_pk * s_pk^2 / qchisq(alpha_c / 2, df_pk)))
  cat(sprintf("pk conf=%.2f: mean=%.12f ci=[%.12f, %.12f] gmean_ci=[%.12f, %.12f] sd_ci=[%.12f, %.12f]\n",
              cl, mean(pk), tt_pk$conf.int[1], tt_pk$conf.int[2],
              exp(tt_g$conf.int[1]), exp(tt_g$conf.int[2]), sd_ci[1], sd_ci[2]))
}
tt_sg <- t.test(signed_v, conf.level = 0.95)
cat(sprintf("signed 95%%: mean=%.12f ci=[%.12f, %.12f]\n", mean(signed_v), tt_sg$conf.int[1], tt_sg$conf.int[2]))

cat("\n=== 8. Power & sample size vs base R 4.5.2 ===\n")
pt_eq <- power.t.test(delta = 0.5, sd = 1, sig.level = 0.05, power = 0.80, type = "two.sample", alternative = "two.sided", strict = TRUE, tol = 1e-12)
pt_eq_pow64 <- power.t.test(n = 64, delta = 0.5, sd = 1, sig.level = 0.05, type = "two.sample", alternative = "two.sided", strict = TRUE)
pt_1side <- power.t.test(delta = 0.5, sd = 1, sig.level = 0.05, power = 0.80, type = "two.sample", alternative = "one.sided", tol = 1e-12)
pt_pair <- power.t.test(delta = 0.5, sd = 1, sig.level = 0.05, power = 0.80, type = "paired", alternative = "two.sided", strict = TRUE, tol = 1e-12)
pt_1s <- power.t.test(delta = 0.5, sd = 1, sig.level = 0.05, power = 0.80, type = "one.sample", alternative = "two.sided", strict = TRUE, tol = 1e-12)
pt_1s_gt <- power.t.test(delta = 0.5, sd = 1, sig.level = 0.05, power = 0.80, type = "one.sample", alternative = "one.sided", tol = 1e-12)
pa_4 <- power.anova.test(groups = 4, between.var = var(c(10, 10, 12, 12)), within.var = 16, sig.level = 0.05, power = 0.80)
pa_4_pow45 <- power.anova.test(groups = 4, n = 45, between.var = var(c(10, 10, 12, 12)), within.var = 16, sig.level = 0.05)
pp_2 <- power.prop.test(p1 = 0.50, p2 = 0.30, sig.level = 0.05, power = 0.80, alternative = "two.sided", strict = TRUE)
pp_2_pow93 <- power.prop.test(n = 93, p1 = 0.50, p2 = 0.30, sig.level = 0.05, alternative = "two.sided", strict = TRUE)
cat(sprintf("power.t.test 2s eq: n=%.12f pow(64)=%.12f\n", pt_eq$n, pt_eq_pow64$power))
cat(sprintf("power.t.test 2s 1-sided: n=%.12f\n", pt_1side$n))
cat(sprintf("power.t.test paired: n=%.12f\n", pt_pair$n))
cat(sprintf("power.t.test 1-sample: n_2s=%.12f n_1s=%.12f\n", pt_1s$n, pt_1s_gt$n))
cat(sprintf("power.anova.test k=4 f=0.25: n=%.12f pow(45)=%.12f\n", pa_4$n, pa_4_pow45$power))
cat(sprintf("power.prop.test 0.5 vs 0.3: n=%.12f pow(93)=%.12f\n", pp_2$n, pp_2_pow93$power))

# Unequal allocation two-sample t (k = n2/n1 = 2, d = 0.5, alpha = 0.05, power = 0.80)
unbal_t_pow <- function(n1, k = 2, d = 0.5, alpha = 0.05) {
  n2 <- k * n1; df <- n1 + n2 - 2
  ncp <- d * sqrt(n1 * n2 / (n1 + n2))
  tc <- qt(1 - alpha / 2, df)
  pt(-tc, df, ncp = ncp) + pt(tc, df, ncp = ncp, lower.tail = FALSE)
}
n1_k2 <- uniroot(function(x) unbal_t_pow(x) - 0.80, c(10, 200), tol = 1e-13)$root
cat(sprintf("unequal-alloc t k=2: n1=%.12f pow(48,96)=%.12f\n", n1_k2, unbal_t_pow(48)))

# Chi-square (w = 0.3, df = 3) & Linear regression (f2 = 0.15, u = 1) via base R pchisq/pf
n_chi2 <- uniroot(function(n) pchisq(qchisq(0.95, 3), 3, ncp = n * 0.3^2, lower.tail = FALSE) - 0.80, c(10, 500), tol = 1e-13)$root
pow_chi2_122 <- pchisq(qchisq(0.95, 3), 3, ncp = 122 * 0.3^2, lower.tail = FALSE)
n_reg <- uniroot(function(n) pf(qf(0.95, 1, n - 2), 1, n - 2, ncp = n * 0.15, lower.tail = FALSE) - 0.80, c(5, 500), tol = 1e-13)$root
pow_reg_55 <- pf(qf(0.95, 1, 53), 1, 53, ncp = 55 * 0.15, lower.tail = FALSE)
# Correlation FisherZ & FisherZWithTCrit (r = 0.3, alpha = 0.05, power = 0.80)
zr <- atanh(0.3)
fz_pow <- function(n) {
  se <- 1 / sqrt(n - 3); zc <- qnorm(0.975)
  pnorm(-zc - zr / se) + pnorm(zc - zr / se, lower.tail = FALSE)
}
n_fz <- uniroot(function(n) fz_pow(n) - 0.80, c(4, 500), tol = 1e-13)$root
fzt_pow <- function(n) {
  rc <- sqrt(qt(0.975, n - 2)^2 / (qt(0.975, n - 2)^2 + n - 2))
  z_rc <- atanh(rc); se <- 1 / sqrt(n - 3)
  mu <- zr + 0.3 / (2 * (n - 1))
  pnorm(-z_rc, mean = mu, sd = se) + pnorm(z_rc, mean = mu, sd = se, lower.tail = FALSE)
}
n_fzt <- uniroot(function(n) fzt_pow(n) - 0.80, c(4, 500), tol = 1e-13)$root
cat(sprintf("chi2 w=0.3 df=3: n=%.12f pow(122)=%.12f\n", n_chi2, pow_chi2_122))
cat(sprintf("reg f2=0.15 u=1: n=%.12f pow(55)=%.12f\n", n_reg, pow_reg_55))
cat(sprintf("corr r=0.3: n_fz=%.12f pow_fz(85)=%.12f n_fzt=%.12f\n", n_fz, fz_pow(85), n_fzt))

cat("\n=== 9. Dunn per-pair R 4.5.2 provenance (equal-n tied & unequal-n tied) ===\n")
dunn_calc <- function(groups) {
  all_x <- unlist(groups)
  N <- length(all_x)
  r <- rank(all_x, ties.method = "average")
  ns <- lengths(groups)
  grp <- rep(seq_along(groups), ns)
  rmean <- as.numeric(tapply(r, grp, mean))
  tab <- table(all_x)
  tie_sum <- sum(tab^3 - tab)
  base_var <- (N * (N + 1) / 12) - tie_sum / (12 * (N - 1))
  k <- length(groups)
  pairs <- combn(k, 2)
  raw_p <- numeric(ncol(pairs))
  rows <- list()
  for (idx in seq_len(ncol(pairs))) {
    i <- pairs[1, idx]; j <- pairs[2, idx]
    diff <- unname(rmean[i] - rmean[j])
    se <- unname(sqrt(base_var * (1 / ns[i] + 1 / ns[j])))
    z <- diff / se
    p <- 2 * pnorm(-abs(z))
    raw_p[idx] <- p
    rows[[idx]] <- c(i = i - 1, j = j - 1, diff = diff, se = se, z = z, raw_p = p)
  }
  p_bonf <- p.adjust(raw_p, method = "bonferroni")
  p_holm <- p.adjust(raw_p, method = "holm")
  p_sidak <- 1 - (1 - raw_p)^length(raw_p)
  list(tie_sum = tie_sum, rmean = rmean, rows = rows, bonf = p_bonf, holm = p_holm, sidak = p_sidak)
}
d_eq <- dunn_calc(list(
  c(12.1, 14.3, 14.3, 15.0, 16.2),
  c(18.5, 19.0, 19.0, 21.4, 22.0),
  c(15.0, 16.8, 17.2, 18.5, 20.1)
))
cat(sprintf("Dunn equal-n tie_sum=%.1f\n", d_eq$tie_sum))
for (idx in seq_along(d_eq$rows)) {
  rw <- d_eq$rows[[idx]]
  cat(sprintf("  (%d,%d) diff=%.12f se=%.12f z=%.12f raw_p=%.12e bonf=%.12e sidak=%.12e holm=%.12e\n",
              rw["i"], rw["j"], rw["diff"], rw["se"], rw["z"], rw["raw_p"], d_eq$bonf[idx], d_eq$sidak[idx], d_eq$holm[idx]))
}
d_un <- dunn_calc(list(
  c(4.0, 5.0, 5.0, 6.0, 7.0),
  c(7.0, 8.0, 8.0, 8.0, 9.0, 10.0, 11.0),
  c(5.0, 6.0, 9.0, 12.0),
  c(11.0, 12.0, 13.0, 14.0, 14.0, 16.0)
))
cat(sprintf("Dunn unequal-n tie_sum=%.1f\n", d_un$tie_sum))
for (idx in seq_along(d_un$rows)) {
  rw <- d_un$rows[[idx]]
  cat(sprintf("  (%d,%d) diff=%.12f se=%.12f z=%.12f raw_p=%.12e bonf=%.12e sidak=%.12e holm=%.12e\n",
              rw["i"], rw["j"], rw["diff"], rw["se"], rw["z"], rw["raw_p"], d_un$bonf[idx], d_un$sidak[idx], d_un$holm[idx]))
}

cat("\n=== 10. Fisher LSD, Newman-Keuls, Tamhane T2, Multiple Row t-tests ===\n")
# 10a. Fisher LSD on ctrl/low/high/mid (unbalanced/balanced pooled MSE)
clhm <- list(Ctrl = ctrl, Low = low, High = high, Mid = mid)
cat(sprintf("Fisher LSD clhm: mse=%.12f df=%d\n", msw_c, df2_c))
tc_lsd <- qt(0.975, df2_c)
for (i in 1:3) {
  for (j in (i + 1):4) {
    diff <- mean(clhm[[i]]) - mean(clhm[[j]])
    se <- sqrt(msw_c * (1 / length(clhm[[i]]) + 1 / length(clhm[[j]])))
    t_s <- diff / se
    p <- 2 * pt(-abs(t_s), df2_c)
    cat(sprintf("  LSD (%d,%d) diff=%.12f se=%.12f t=%.12f p=%.12e ci=[%.12f, %.12f]\n",
                i - 1, j - 1, diff, se, t_s, p, diff - tc_lsd * se, diff + tc_lsd * se))
  }
}

# 10b. Newman-Keuls on 4-group dataset where step-down monotonicity clamping triggers!
# Means: A=10.0, B=11.3, C=12.6, D=14.1, n=(5,5,5,5), MSE=4.0, df=16 -> span 4 (A,D) p > span 2 or border
snk_g <- list(
  G0 = c(8.0, 9.5, 10.0, 10.5, 12.0),
  G1 = c(9.5, 10.5, 11.5, 12.0, 13.0),
  G2 = c(10.5, 12.0, 12.8, 13.2, 14.5),
  G3 = c(12.2, 13.4, 14.0, 14.8, 16.1)
)
snk_df <- data.frame(y = unlist(snk_g), g = factor(rep(names(snk_g), lengths(snk_g))))
aov_snk <- summary(aov(y ~ g, data = snk_df))[[1]]
mse_snk <- aov_snk$`Mean Sq`[2]; df_snk <- aov_snk$Df[2]
cat(sprintf("Newman-Keuls snk_g: mse=%.12f df=%d\n", mse_snk, df_snk))
snk_ms <- sapply(snk_g, mean); snk_ns <- lengths(snk_g)
ord <- order(snk_ms)
p_mat <- matrix(1, 4, 4)
for (span in 4:2) {
  for (lo in 1:(5 - span)) {
    hi <- lo + span - 1
    ii <- ord[lo]; jj <- ord[hi]
    se_q <- sqrt(0.5 * mse_snk * (1 / snk_ns[ii] + 1 / snk_ns[jj]))
    q_s <- abs(snk_ms[jj] - snk_ms[ii]) / se_q
    rp <- q_sf_exact(q_s, span, df_snk)
    adj <- rp
    if (lo > 1) adj <- max(adj, p_mat[lo - 1, hi])
    if (hi < 4) adj <- max(adj, p_mat[lo, hi + 1])
    p_mat[lo, hi] <- adj
  }
}
pos_of <- integer(4); pos_of[ord] <- 1:4
for (i in 1:3) {
  for (j in (i + 1):4) {
    se_q <- sqrt(0.5 * mse_snk * (1 / snk_ns[i] + 1 / snk_ns[j]))
    q_s <- abs(snk_ms[i] - snk_ms[j]) / se_q
    raw_p <- q_sf_exact(q_s, 2, df_snk)
    lo <- min(pos_of[i], pos_of[j]); hi <- max(pos_of[i], pos_of[j])
    cat(sprintf("  SNK (%d,%d) q=%.12f raw_p=%.12e adj_p=%.12e\n", i - 1, j - 1, q_s, raw_p, p_mat[lo, hi]))
  }
}

# Unequal-n Newman-Keuls where enclosing range (span=3/4) has LARGER p than inner span, triggering step-down clamping!
# G0 (n=2, mean=10), G1 (n=20, mean=11), G2 (n=20, mean=14), G3 (n=2, mean=15), MSE=4.0, df=40
snk_c_ns <- c(2, 20, 20, 2); snk_c_ms <- c(10.0, 11.0, 14.0, 15.0); mse_c <- 4.0; df_c <- 40
p_mc <- matrix(1, 4, 4)
for (span in 4:2) {
  for (lo in 1:(5 - span)) {
    hi <- lo + span - 1
    se_q <- sqrt(0.5 * mse_c * (1 / snk_c_ns[lo] + 1 / snk_c_ns[hi]))
    q_s <- abs(snk_c_ms[hi] - snk_c_ms[lo]) / se_q
    rp <- q_sf_exact(q_s, span, df_c)
    adj <- rp
    if (lo > 1) adj <- max(adj, p_mc[lo - 1, hi])
    if (hi < 4) adj <- max(adj, p_mc[lo, hi + 1])
    p_mc[lo, hi] <- adj
  }
}
cat("Newman-Keuls step-down clamped (unequal n=(2,20,20,2), MSE=4, df=40):\n")
for (i in 1:3) {
  for (j in (i + 1):4) {
    se_q <- sqrt(0.5 * mse_c * (1 / snk_c_ns[i] + 1 / snk_c_ns[j]))
    q_s <- abs(snk_c_ms[i] - snk_c_ms[j]) / se_q
    raw_p <- q_sf_exact(q_s, 2, df_c)
    cat(sprintf("  SNK_clamp (%d,%d) q=%.12f raw_p=%.12e adj_p=%.12e\n", i - 1, j - 1, q_s, raw_p, p_mc[i, j]))
  }
}

# 10c. Tamhane T2 on heteroscedastic `het` (m = 6)
alpha_sid <- 1 - (1 - 0.05)^(1 / m)
cat("Tamhane T2 on het:\n")
for (i in 1:(k - 1)) {
  for (j in (i + 1):k) {
    wt <- t.test(het[[i]], het[[j]], var.equal = FALSE)
    diff <- ms[i] - ms[j]; se <- abs(diff / unname(wt$statistic)); df <- unname(wt$parameter)
    rp <- unname(wt$p.value); ap <- 1 - (1 - rp)^m
    tc <- qt(1 - alpha_sid / 2, df)
    cat(sprintf("  T2 (%d,%d) t=%.12f df=%.12f raw_p=%.12e adj_p=%.12e ci=[%.12f, %.12f]\n",
                i - 1, j - 1, unname(wt$statistic), df, rp, ap, diff - tc * se, diff + tc * se))
  }
}

# 10d. Multiple row t-tests (6 rows, Student & Welch, Bonf/Holm/BH/BY/Holm-Sidak)
mr_a <- list(
  c(10.0, 10.5, 11.0, 10.2, 10.4),
  c(5.0, 5.2, 4.9, 5.1, 5.3),
  c(20.0, 20.4, 19.6, 20.1, 19.9),
  c(12.0, 12.8, 11.5, 12.4, 12.1),
  c(8.0, 8.6, 7.5, 8.2, 8.1),
  c(15.0, 15.2, 14.9, 15.1, 14.8)
)
mr_b <- list(
  c(10.1, 10.6, 10.9, 10.3, 10.5),
  c(8.1, 8.5, 7.9, 8.2, 8.3),
  c(19.7, 20.5, 19.9, 20.2, 20.1),
  c(14.5, 15.2, 13.9, 14.8, 14.1),
  c(9.2, 9.8, 8.9, 9.5, 9.0),
  c(15.9, 16.4, 15.6, 16.0, 15.7)
)
for (welch_flag in c(FALSE, TRUE)) {
  tag <- if (welch_flag) "Welch" else "Student"
  res_list <- lapply(seq_along(mr_a), function(i) t.test(mr_a[[i]], mr_b[[i]], var.equal = !welch_flag))
  rp <- sapply(res_list, function(r) r$p.value)
  bonf <- p.adjust(rp, "bonferroni")
  holm <- p.adjust(rp, "holm")
  bh <- p.adjust(rp, "BH")
  by <- p.adjust(rp, "BY")
  cat(sprintf("Multiple row t-tests (%s):\n", tag))
  for (i in seq_along(mr_a)) {
    r <- res_list[[i]]
    cat(sprintf("  row %d: t=%.12f df=%.12f raw_p=%.12e bonf=%.12e holm=%.12e bh=%.12e by=%.12e\n",
                i - 1, unname(r$statistic), unname(r$parameter), rp[i], bonf[i], holm[i], bh[i], by[i]))
  }
}

cat("\n=== 11. Paired sign test vs stats::binom.test ===\n")
sign_a <- c(12.5, 14.1, 11.8, 15.0, 13.2, 16.4, 10.9, 14.8, 13.5, 12.0, 15.2, 14.0, 11.5)
sign_b <- c(10.2, 13.0, 12.1, 12.5, 13.2, 14.0,  9.5, 12.2, 11.9, 12.0, 13.0, 11.8, 10.1)
# diffs: 10 pos, 1 neg, 2 exact zeros (n_nonzero = 11)
d_s <- sign_a - sign_b
n_pos <- sum(d_s > 0); n_neg <- sum(d_s < 0); n_nz <- n_pos + n_neg
bt_2s <- binom.test(n_pos, n_nz, p = 0.5, alternative = "two.sided")
bt_1s <- binom.test(min(n_pos, n_neg), n_nz, p = 0.5, alternative = "less")
cat(sprintf("sign_test n_pos=%d n_neg=%d n_ties=%d p_2s=%.15f p_1s=%.15f med=%.12f\n",
            n_pos, n_neg, sum(d_s == 0), bt_2s$p.value, bt_1s$p.value, median(d_s)))
# Exact binomial order-statistic median CI (Conover 1999) for n = 13 at 95%
sd_sorted <- sort(d_s); n_tot <- length(sd_sorted)
# Find largest k >= 0 with 2 * pbinom(k, n_tot, 0.5) <= 0.05 -> order stats k+1 and n_tot-k (1-based)
k_lo <- max(which(2 * pbinom(0:floor(n_tot / 2), n_tot, 0.5) <= 0.05)) - 1
actual_cov <- 1 - 2 * pbinom(k_lo, n_tot, 0.5)
cat(sprintf("sign_test median CI: idx=(%d,%d) bounds=[%.12f, %.12f] actual_conf=%.12f%%\n",
            k_lo + 1, n_tot - k_lo, sd_sorted[k_lo + 1], sd_sorted[n_tot - k_lo], 100 * actual_cov))

cat("\n=== 12. Baptista-Pike (1977) exact conditional OR CI via R uniroot ===\n")
bp_p <- function(a, b, c, d, log_or) {
  r1 <- a + b; r2 <- c + d; c1 <- a + c
  lo <- max(0L, c1 - r2); hi <- min(r1, c1)
  xs <- lo:hi
  # Log noncentral hypergeometric kernel using base R lchoose
  lw <- lchoose(r1, xs) + lchoose(r2, c1 - xs) + xs * log_or
  lw <- lw - max(lw)
  probs <- exp(lw) / sum(exp(lw))
  p_obs <- probs[xs == a]
  sum(probs[probs <= p_obs * (1 + 1e-12)])
}
bp_ci <- function(a, b, c, d, alpha = 0.05) {
  r1 <- a + b; r2 <- c + d; c1 <- a + c
  lo_x <- max(0L, c1 - r2); hi_x <- min(r1, c1)
  # Anchor at conditional MLE or observed OR
  or_hint <- if (a == 0 || d == 0) 0.5 / max(b * c, 1) else if (b == 0 || c == 0) (a + 0.5) * (d + 0.5) / ((b + 0.5) * (c + 0.5)) else (a * d) / (b * c)
  u0 <- log(or_hint)
  if (bp_p(a, b, c, d, u0) < alpha) {
    u0 <- optimize(function(u) bp_p(a, b, c, d, u), c(-15, 15), maximum = TRUE)$maximum
  }
  lo <- if (a <= lo_x) 0 else exp(uniroot(function(u) bp_p(a, b, c, d, u) - alpha, c(u0 - 20, u0), tol = 1e-13)$root)
  hi <- if (a >= hi_x) Inf else exp(uniroot(function(u) bp_p(a, b, c, d, u) - alpha, c(u0, u0 + 20), tol = 1e-13)$root)
  c(lo, hi)
}
bp_cases <- list(
  list(a = 14, b = 6, c = 5, d = 15, alpha = 0.05),
  list(a = 14, b = 6, c = 5, d = 15, alpha = 0.01),
  list(a = 12, b = 8, c = 4, d = 16, alpha = 0.01),
  list(a = 8,  b = 2, c = 3, d = 12, alpha = 0.05),
  list(a = 0,  b = 10, c = 5, d = 10, alpha = 0.05),
  list(a = 10, b = 0, c = 5, d = 10, alpha = 0.05)
)
for (cs in bp_cases) {
  ci <- bp_ci(cs$a, cs$b, cs$c, cs$d, cs$alpha)
  cat(sprintf("BP (%d,%d,%d,%d) alpha=%.2f: lo=%.12f hi=%.12f\n", cs$a, cs$b, cs$c, cs$d, cs$alpha, ci[1], ci[2]))
}

cat("\n=== 13. Fixed-centre Lloyd k-means vs stats::kmeans(algorithm='Lloyd') ===\n")
km_x <- rbind(
  c(0.2, 0.5), c(0.8, 0.1), c(1.1, 0.9), c(0.4, 1.3), c(1.5, 0.4),
  c(5.2, 4.8), c(5.9, 5.5), c(4.6, 5.3), c(6.1, 4.9), c(5.4, 6.0),
  c(1.0, 6.2), c(1.6, 5.8), c(0.7, 6.9), c(2.0, 6.5), c(1.3, 7.1)
)
km_init <- rbind(c(0.0, 0.0), c(3.0, 3.0), c(2.5, 6.0))
km_fit <- kmeans(km_x, centers = km_init, iter.max = 100, algorithm = "Lloyd")
cat("Lloyd assignments (0-based):", paste(km_fit$cluster - 1, collapse = ","), "\n")
cat(sprintf("Lloyd tot.withinss=%.12f\n", km_fit$tot.withinss))
for (ci in 1:3) {
  cat(sprintf("  center[%d] = (%.12f, %.12f)\n", ci - 1, km_fit$centers[ci, 1], km_fit$centers[ci, 2]))
}

cat("\n=== 14. Hyndman-Fan Type-6 quantiles ===\n")
q_probs <- c(0.0, 0.05, 0.10, 0.25, 0.50, 0.75, 0.90, 0.95, 1.0)
q_samples <- list(
  odd7 = c(2.1, 4.5, 5.0, 7.2, 9.8, 12.4, 18.0),
  even8 = c(2.4, 3.1, 3.8, 4.2, 5.0, 6.5, 8.2, 11.4),
  small4 = c(10.0, 20.0, 35.0, 80.0),
  tied9 = c(1.0, 2.0, 2.0, 2.0, 5.0, 7.0, 7.0, 9.0, 15.0),
  n15 = c(1.2, 2.5, 3.1, 4.0, 5.5, 6.2, 7.8, 9.1, 10.4, 12.0, 14.5, 16.8, 19.2, 22.0, 27.5)
)
for (nm in names(q_samples)) {
  qs <- quantile(q_samples[[nm]], probs = q_probs, type = 6, names = FALSE)
  cat(sprintf("Type6 %s: %s\n", nm, paste(sprintf("%.12f", qs), collapse = ", ")))
}

cat("\n=== 15. Exact binom.test vs stats::binom.test (+ Wilson & Agresti-Coull) ===\n")
b_cases <- list(
  c(k = 18, n = 25, p0 = 0.5),
  c(k = 7,  n = 20, p0 = 0.2),
  c(k = 2,  n = 15, p0 = 0.35),
  c(k = 0,  n = 12, p0 = 0.25),
  c(k = 12, n = 12, p0 = 0.80)
)
for (bc in b_cases) {
  k0 <- bc["k"]; n0 <- bc["n"]; p0 <- bc["p0"]
  b2 <- binom.test(k0, n0, p = p0, alternative = "two.sided")
  bl <- binom.test(k0, n0, p = p0, alternative = "less")
  bg <- binom.test(k0, n0, p = p0, alternative = "greater")
  pw <- prop.test(k0, n0, correct = FALSE)
  z <- qnorm(0.975); z2 <- z^2; nt <- n0 + z2; pt_ac <- (k0 + 0.5 * z2) / nt
  ac_lo <- max(0, pt_ac - z * sqrt(pt_ac * (1 - pt_ac) / nt))
  ac_hi <- min(1, pt_ac + z * sqrt(pt_ac * (1 - pt_ac) / nt))
  cat(sprintf("binom(%d,%d,p0=%.2f): p_lo=%.15f p_hi=%.15f p_sterne=%.15f p_2tail=%.15f\n",
              k0, n0, p0, bl$p.value, bg$p.value, b2$p.value, min(1, 2 * min(bl$p.value, bg$p.value))))
  cat(sprintf("  CP=[%.12f, %.12f] Wilson=[%.12f, %.12f] AC=[%.12f, %.12f]\n",
              b2$conf.int[1], b2$conf.int[2], pw$conf.int[1], pw$conf.int[2], ac_lo, ac_hi))
}

cat("\n=== 16. Column ROUT RSDR, robust_center, t & raw p ===\n")
rout_r <- function(x, Q_pct = 1.0) {
  n <- length(x)
  p68 <- 0.682689492
  rsdr_fn <- function(c_val) {
    ar <- sort(abs(x - c_val))
    as.numeric(quantile(ar, probs = p68, type = 7, names = FALSE)) * (n / (n - 1))
  }
  center <- as.numeric(quantile(x, probs = 0.5, type = 6, names = FALSE))
  sc <- rsdr_fn(center)
  if (sc > 0) {
    for (iter in 1:25) {
      r <- (x - center) / sc
      w <- 1 / (1 + r^2)
      nxt <- sum(w * x) / sum(w)
      sh <- abs(nxt - center)
      center <- nxt
      nr <- rsdr_fn(center)
      if (nr > 0) sc <- nr
      if (sh <= 1e-12 * max(sc, 1.0)) break
    }
  }
  res <- abs(x - center)
  rsdr <- rsdr_fn(center)
  t_s <- res / rsdr
  p_s <- 2 * pt(-t_s, df = n - 1)
  list(center = center, rsdr = rsdr, t = t_s, p = p_s)
}
x_rout1 <- c(10.2, 9.8, 10.5, 11.1, 9.4, 10.0, 10.7, 9.9, 10.3, 10.1, 9.6, 10.8, 35.0, 42.0)
rr1 <- rout_r(x_rout1, 1.0)
cat(sprintf("ROUT x_rout1: center=%.12f rsdr=%.12f\n", rr1$center, rr1$rsdr))
for (idx in c(13, 14)) {
  cat(sprintf("  outlier idx=%d val=%.2f t=%.12f p=%.12e\n", idx - 1, x_rout1[idx], rr1$t[idx], rr1$p[idx]))
}
x_rout2 <- c(50.1, 49.4, 51.2, 50.8, 48.9, 50.5, 49.7, 51.0, 50.3, 49.6, 88.0, 12.5)
rr2 <- rout_r(x_rout2, 1.0)
cat(sprintf("ROUT x_rout2: center=%.12f rsdr=%.12f\n", rr2$center, rr2$rsdr))
for (idx in c(11, 12)) {
  cat(sprintf("  outlier idx=%d val=%.2f t=%.12f p=%.12e\n", idx - 1, x_rout2[idx], rr2$t[idx], rr2$p[idx]))
}

cat("\n=== 17. norm.qq Blom (1958) convention check ===\n")
qq8 <- c(2.4, 3.1, 3.8, 4.2, 5.0, 6.5, 8.2, 11.4)
z_qq8_r <- qqnorm(qq8, plot.it = FALSE)$x # For n <= 10, R ppoints uses a = 3/8 (Blom)
exp_qq8_norm <- mean(qq8) + sd(qq8) * z_qq8_r
ln_qq8 <- log(qq8)
exp_qq8_ln <- mean(ln_qq8) + sd(ln_qq8) * z_qq8_r
cat("QQ n=8 normal (matches R qqnorm):", paste(sprintf("%.12f", exp_qq8_norm), collapse = ", "), "\n")
cat("QQ n=8 lognormal:", paste(sprintf("%.12f", exp_qq8_ln), collapse = ", "), "\n")
qq15 <- c(10.1, 10.8, 11.2, 11.5, 11.9, 12.0, 12.3, 12.5, 12.7, 13.0, 13.4, 13.8, 14.1, 14.7, 15.6)
z_blom15 <- qnorm((1:15 - 3/8) / (15 + 1/4))
z_r15 <- qnorm(ppoints(15))
cat("QQ n=15 Blom [1,8,15]:", sprintf("%.12f, %.12f, %.12f", mean(qq15) + sd(qq15) * z_blom15[1], mean(qq15) + sd(qq15) * z_blom15[8], mean(qq15) + sd(qq15) * z_blom15[15]), "\n")
cat("QQ n=15 R ppoints(a=0.5) [1,15]:", sprintf("%.12f, %.12f", mean(qq15) + sd(qq15) * z_r15[1], mean(qq15) + sd(qq15) * z_r15[15]), "\n")

cat("\n=== 18. Passing-Bablok & Barnard exact R 4.5.2 cross-check ===\n")
# 18a. Passing-Bablok PB16 + K-offset n=8 + quadratic CUSUM n=36
pb_r <- function(x, y, conf = 0.95) {
  n <- length(x)
  slopes <- c()
  for (i in 1:(n - 1)) {
    for (j in (i + 1):n) {
      dx <- x[j] - x[i]; dy <- y[j] - y[i]
      if (dx == 0 && dy == 0) next
      s <- if (dx == 0) Inf * sign(dy) else dy / dx
      if (abs(s + 1.0) <= 1e-12) next
      slopes <- c(slopes, s)
    }
  }
  slopes <- sort(slopes)
  N_s <- length(slopes)
  K <- sum(slopes < -1.0)
  med_idx_lo <- floor((N_s + 1) / 2) + K
  med_idx_hi <- ceiling((N_s + 1) / 2) + K
  b <- 0.5 * (slopes[med_idx_lo] + slopes[med_idx_hi])
  z <- qnorm(1 - (1 - conf) / 2)
  C_gamma <- z * sqrt(n * (n - 1) * (2 * n + 5) / 18)
  M1 <- round((N_s - C_gamma) / 2)
  M2 <- N_s - M1 + 1
  b_lo <- slopes[M1 + K]; b_hi <- slopes[M2 + K]
  a <- median(y - b * x)
  a_lo <- median(y - b_hi * x); a_hi <- median(y - b_lo * x)
  # CUSUM linearity
  D <- (y - a - b * x) / sqrt(1 + b^2)
  rank_score <- x + y / b # Wait: Passing-Bablok orders by D_i = (y_i + x_i/b - a) / sqrt(1 + 1/b^2)
  ord_r <- order(x + (y - a) / b)
  D_ord <- D[ord_r]
  L <- sum(D_ord > 1e-12); l <- sum(D_ord < -1e-12)
  scores <- ifelse(D_ord > 1e-12, sqrt(l / L), ifelse(D_ord < -1e-12, -sqrt(L / l), 0))
  H <- max(abs(cumsum(scores)))
  stat <- H / sqrt(L + l)
  k_terms <- 1:200
  p_ks <- min(1, max(0, 2 * sum((-1)^(k_terms - 1) * exp(-2 * (k_terms^2) * (stat^2)))))
  list(N = N_s, K = K, b = b, b_ci = c(b_lo, b_hi), a = a, a_ci = c(a_lo, a_hi), H = H, stat = stat, p = p_ks)
}
pb16 <- pb_r(c(1,2,3,4,5,6,7,8,9,10,12,14,16,18,20,22), c(3.4,5.1,6.8,9.3,10.6,13.1,14.9,17.4,18.8,21.3,24.7,29.2,32.8,37.5,41.0,45.2))
cat(sprintf("PB16 R: N=%d K=%d b=%.12f b_ci=[%.12f, %.12f] a=%.12f a_ci=[%.12f, %.12f] H=%.12f stat=%.12f p=%.12f\n",
            pb16$N, pb16$K, pb16$b, pb16$b_ci[1], pb16$b_ci[2], pb16$a, pb16$a_ci[1], pb16$a_ci[2], pb16$H, pb16$stat, pb16$p))
# Non-zero K offset (positive slope with 2 pairs having S_ij < -1)
pb_k <- pb_r(c(1.0, 2.0, 3.0, 4.5, 5.0, 6.0, 7.5, 9.0, 11.0, 13.0),
             c(2.1, 4.5, 5.8, 11.2, 9.0, 12.4, 15.1, 18.3, 22.5, 26.2))
cat(sprintf("PB_Kpos R: N=%d K=%d b=%.12f b_ci=[%.12f, %.12f] a=%.12f a_ci=[%.12f, %.12f] H=%.12f p=%.12f\n",
            pb_k$N, pb_k$K, pb_k$b, pb_k$b_ci[1], pb_k$b_ci[2], pb_k$a, pb_k$a_ci[1], pb_k$a_ci[2], pb_k$H, pb_k$p))
qx <- 1:36; qy <- 0.25 * qx^2 + 0.5 * qx + 1.0
pb_q <- pb_r(qx, qy)
cat(sprintf("PB_quad36 R: H=%.12f stat=%.12f p=%.15f\n", pb_q$H, pb_q$stat, pb_q$p))

# 18b. Barnard exact ScorePooled in base R 4.5.2 (Suissa & Shuster 1985)
barnard_r <- function(a, b, c, d) {
  n1 <- a + b; n2 <- c + d; N <- n1 + n2
  score_z <- function(i, j) {
    p1 <- i / n1; p2 <- j / n2; p <- (i + j) / N
    if (p == 0 || p == 1) return(0)
    (p1 - p2) / sqrt(p * (1 - p) * (1 / n1 + 1 / n2))
  }
  z_obs <- abs(score_z(a, c))
  grid <- expand.grid(i = 0:n1, j = 0:n2)
  zs <- abs(mapply(score_z, grid$i, grid$j))
  rej <- grid[zs >= z_obs - 1e-12, ]
  p_of_pi <- function(pi_val) sum(dbinom(rej$i, n1, pi_val) * dbinom(rej$j, n2, pi_val))
  pis <- seq(1e-6, 1 - 1e-6, length.out = 2000)
  vals <- sapply(pis, p_of_pi)
  best_i <- which.max(vals)
  lo <- pis[max(1, best_i - 2)]; hi <- pis[min(length(pis), best_i + 2)]
  opt <- optimize(p_of_pi, c(lo, hi), maximum = TRUE, tol = 1e-13)
  c(p = opt$objective, pi = opt$maximum)
}
for (tbl in list(c(3,1,1,3), c(1,7,6,2), c(14,6,5,15), c(1,19,6,14), c(3,0,0,12))) {
  br <- barnard_r(tbl[1], tbl[2], tbl[3], tbl[4])
  cat(sprintf("Barnard R (%d,%d,%d,%d): p=%.15f pi=%.12f\n", tbl[1], tbl[2], tbl[3], tbl[4], br["p"], br["pi"]))
}

cat("\n=== 19. Shapiro-Wilk n=12, n=25, & Royston (1995 AS R94) n=25 ===\n")
x_norm12 <- c(10.2, 9.8, 10.5, 11.1, 9.4, 10.0, 10.7, 9.9, 10.3, 10.1, 9.6, 10.8)
skew25 <- c(0.1, 0.1, 0.1, 0.2, 0.2, 0.2, 0.3, 0.4, 0.5, 0.5, 0.6, 0.7, 0.8, 1.0, 1.1, 1.3, 1.5, 1.8, 2.1, 2.5, 3.0, 3.5, 4.0, 5.0, 6.0)
royston25 <- c(0.139, 0.157, 0.175, 0.256, 0.344, 0.413, 0.503, 0.577, 0.614, 0.655,
               0.954, 1.392, 1.557, 1.648, 1.690, 1.994, 2.174, 2.206, 3.245, 3.510,
               3.571, 4.354, 4.980, 6.084, 8.351)
for (item in list(list("x_norm12", x_norm12), list("skew25", skew25), list("royston25", royston25))) {
  sw <- shapiro.test(item[[2]])
  cat(sprintf("shapiro.test %s: W=%.16f p=%.16e\n", item[[1]], sw$statistic, sw$p.value))
}

cat("\n=== 21. NIST GEAR.DAT worked example (100 rows, 10 batches) ===\n")
gear_batches <- list(
  c(1.006, 0.996, 0.998, 1.000, 0.992, 0.993, 1.002, 0.999, 0.994, 1.000),
  c(0.998, 1.006, 1.000, 1.002, 0.997, 0.998, 0.996, 1.000, 1.006, 0.988),
  c(0.991, 0.987, 0.997, 0.999, 0.995, 0.994, 1.000, 0.999, 0.996, 0.996),
  c(1.005, 1.002, 0.994, 1.000, 0.995, 0.994, 0.998, 0.996, 1.002, 0.996),
  c(0.998, 0.998, 0.982, 0.990, 1.002, 0.984, 0.996, 0.993, 0.980, 0.996),
  c(1.009, 1.013, 1.009, 0.997, 0.988, 1.002, 0.995, 0.998, 0.981, 0.996),
  c(0.990, 1.004, 0.996, 1.001, 0.998, 1.000, 1.018, 1.010, 0.996, 1.002),
  c(0.998, 1.000, 1.006, 1.000, 1.002, 0.996, 0.998, 0.996, 1.002, 1.006),
  c(1.002, 0.998, 0.996, 0.995, 0.996, 1.004, 1.004, 0.998, 0.999, 0.991),
  c(0.991, 0.995, 0.984, 0.994, 0.997, 0.997, 0.991, 0.998, 1.004, 0.997)
)
gear_y <- unlist(gear_batches)
gear_g <- factor(rep(1:10, each = 10))
bt_gear <- bartlett.test(gear_y ~ gear_g)
lev_med_gear <- levene_calc(gear_batches, TRUE)
lev_mean_gear <- levene_calc(gear_batches, FALSE)
aov_gear <- summary(aov(gear_y ~ gear_g))[[1]]
cat(sprintf("GEAR Bartlett: T=%.12f df=%d p=%.12e\n", bt_gear$statistic, bt_gear$parameter, bt_gear$p.value))
cat(sprintf("GEAR Levene(median/BF): W=%.12f df1=%d df2=%d p=%.12e\n",
            lev_med_gear["F"], lev_med_gear["df1"], lev_med_gear["df2"], lev_med_gear["p"]))
cat(sprintf("GEAR Levene(mean): W=%.12f df1=%d df2=%d p=%.12e\n",
            lev_mean_gear["F"], lev_mean_gear["df1"], lev_mean_gear["df2"], lev_mean_gear["p"]))
cat(sprintf("GEAR One-way ANOVA: F=%.12f df1=%d df2=%d p=%.12e\n",
            aov_gear$`F value`[1], aov_gear$Df[1], aov_gear$Df[2], aov_gear$`Pr(>F)`[1]))

