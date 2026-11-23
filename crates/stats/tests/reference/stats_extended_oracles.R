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
