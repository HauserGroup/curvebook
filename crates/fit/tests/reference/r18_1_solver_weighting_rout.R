#!/usr/bin/env Rscript
# Independent R 4.5.2 oracle generator for R18.1 (§1.4 Nonlinear Solver, Weighting & Method Extensions).
# References:
# - Carroll & Ruppert (1988) Transformation and Weighting in Regression
# - Motulsky & Christopoulos (2004) Fitting Models to Biological Data
# - Motulsky & Brown (2006) BMC Bioinformatics 7:123
# - Burnham & Anderson (2002) Model Selection and Multimodel Inference

options(digits = 15)

irls_predicted_y <- function(df, start, power_k, tol = 1e-11, max_iter = 100) {
  ctrl <- nls.control(tol = 1e-10, maxiter = 500)
  fit <- nls(y ~ (Y0 - Plateau) * exp(-K * x) + Plateau, data = df, start = start, algorithm = "port", control = ctrl)
  prev_wss <- sum(residuals(fit)^2)
  for (iter in seq_len(max_iter)) {
    yhat <- as.numeric(predict(fit))
    w <- 1 / (abs(yhat)^power_k)
    fit <- nls(y ~ (Y0 - Plateau) * exp(-K * x) + Plateau, data = df, start = as.list(coef(fit)),
               weights = w, algorithm = "port", control = ctrl)
    yhat_new <- as.numeric(predict(fit))
    w_new <- 1 / (abs(yhat_new)^power_k)
    wss <- sum(w_new * (df$y - yhat_new)^2)
    if (abs(prev_wss - wss) / max(1, abs(prev_wss)) < tol) {
      return(list(coef = coef(fit), se = summary(fit)$coefficients[, "Std. Error"], wss = wss, iter = iter))
    }
    prev_wss <- wss
  }
  stop("IRLS did not converge")
}

cat("=== 1. Curve-predicted IRLS weighting (PredictedY) ===\n")
x_decay <- c(0, 0, 1, 2, 3, 4, 5, 6, 8, 10, 10, 12, 12)
# Includes y_obs == 0.0 at x = 10 while replicate + x = 12 anchor Plateau > 10 (y_hat > 0 everywhere)
y_decay <- c(114.2, 116.8, 82.1, 59.5, 44.0, 34.2, 27.5, 23.1, 18.4, 0.0, 32.2, 15.8, 16.4)
df_decay <- data.frame(x = x_decay, y = y_decay)
start_decay <- list(Y0 = 115, Plateau = 15, K = 0.4)

for (k in c(2.0, 1.0, 1.5)) {
  res <- irls_predicted_y(df_decay, start_decay, k)
  cat(sprintf("k=%.1f: Y0=%.12f Plateau=%.12f K=%.12f WSS=%.12f SE_Y0=%.12f SE_Pl=%.12f SE_K=%.12f\n",
              k, res$coef["Y0"], res$coef["Plateau"], res$coef["K"], res$wss,
              res$se["Y0"], res$se["Plateau"], res$se["K"]))
}

cat("\n=== 2. ReplicateFitMode: IndividualReplicates vs RowMeansOnly (Michaelis-Menten) ===\n")
x_mm_rows <- c(1, 2, 4, 8, 16, 32)
y_mm_mat <- rbind(
  c(19.2, 21.1, 19.7),
  c(32.5, 34.0, 33.4),
  c(51.2, 49.0, 50.1),
  c(66.1, 68.2, 65.8),
  c(79.5, 81.2, 79.9),
  c(88.0, 89.6, 88.8)
)
df_mm_ind <- data.frame(x = rep(x_mm_rows, each = 3), y = as.vector(t(y_mm_mat)))
df_mm_means <- data.frame(x = x_mm_rows, y = rowMeans(y_mm_mat))
fit_mm_ind <- nls(y ~ Vmax * x / (Km + x), data = df_mm_ind, start = list(Vmax = 100, Km = 4),
                  algorithm = "port", control = nls.control(tol = 1e-10, maxiter = 500))
fit_mm_means <- nls(y ~ Vmax * x / (Km + x), data = df_mm_means, start = list(Vmax = 100, Km = 4),
                    algorithm = "port", control = nls.control(tol = 1e-10, maxiter = 500))
cat(sprintf("Individual: Vmax=%.12f Km=%.12f RSS=%.12f df=%d\n",
            coef(fit_mm_ind)["Vmax"], coef(fit_mm_ind)["Km"], sum(residuals(fit_mm_ind)^2), df.residual(fit_mm_ind)))
cat(sprintf("RowMeans:   Vmax=%.12f Km=%.12f RSS=%.12f df=%d\n",
            coef(fit_mm_means)["Vmax"], coef(fit_mm_means)["Km"], sum(residuals(fit_mm_means)^2), df.residual(fit_mm_means)))

cat("\n=== 3. ROUT IdentifyAndCountOnly (OLS retains outlier) ===\n")
x_out <- 1:14
y_out <- 2.0 * x_out + 5.0 + c(0.1, -0.1, 0.05, -0.08, 0.12, -0.05, 0.07, -0.09, 0.06, -0.04, 0.08, -0.07, 0.03, 40.0)
fit_ols_all <- lm(y_out ~ x_out)
fit_ols_clean <- lm(y_out[-14] ~ x_out[-14])
cat(sprintf("AllPts:   Intercept=%.12f Slope=%.12f RSS=%.12f\n",
            coef(fit_ols_all)[1], coef(fit_ols_all)[2], sum(residuals(fit_ols_all)^2)))
cat(sprintf("CleanPts: Intercept=%.12f Slope=%.12f RSS=%.12f\n",
            coef(fit_ols_clean)[1], coef(fit_ols_clean)[2], sum(residuals(fit_ols_clean)^2)))

cat("\n=== 4. Union-of-outliers Extra-SS F and AICc (straight_line vs quadratic on I \\ (O1 U O2)) ===\n")
# Mildly curved dataset with two planted outliers: index 3 (extreme for both) and index 12 (moderate)
x_cmp <- seq(0, 10, length.out = 21)
y_cmp <- 2.0 + 1.5 * x_cmp + 0.18 * x_cmp^2 +
  c(0.05, -0.04, 0.03, -0.06, 0.02, -0.03, 0.04, -0.05, 0.06, -0.02,
    0.03, -0.04, 0.05, -0.03, 0.02, -0.05, 0.04, -0.02, 0.03, -0.04, 0.05)
# Plant outliers at R 1-based indices 4 (x=1.5) and 19 (x=9.0)
y_cmp[4] <- y_cmp[4] + 25.0
y_cmp[19] <- y_cmp[19] - 3.2
keep <- setdiff(seq_along(x_cmp), c(4, 19))
m1 <- lm(y_cmp[keep] ~ x_cmp[keep])
m2 <- lm(y_cmp[keep] ~ x_cmp[keep] + I(x_cmp[keep]^2))
ss1 <- sum(residuals(m1)^2)
ss2 <- sum(residuals(m2)^2)
df1 <- df.residual(m1)
df2 <- df.residual(m2)
f_stat <- ((ss1 - ss2) / (df1 - df2)) / (ss2 / df2)
p_val <- pf(f_stat, df1 - df2, df2, lower.tail = FALSE)
n_keep <- length(keep)
aicc <- function(ss, k_free, n) {
  k <- k_free + 1
  n * log(ss / n) + 2 * k + 2 * k * (k + 1) / (n - k - 1)
}
aicc1 <- aicc(ss1, 2, n_keep)
aicc2 <- aicc(ss2, 3, n_keep)
cat(sprintf("UnionKeep(n=%d): SS1=%.12f df1=%d SS2=%.12f df2=%d F=%.12f p=%.12e dAICc=%.12f\n",
            n_keep, ss1, df1, ss2, df2, f_stat, p_val, aicc2 - aicc1))

cat("\n=== 5. Multi-response MV (M=2 responses x G=2 groups = 4 curves) ===\n")
x_mv <- c(1, 2, 3, 4, 5, 6)
y1_ctrl <- 1.0 + 2.0 * x_mv + c(0.1, -0.1, 0.05, -0.05, 0.08, -0.08)
y1_trt  <- 2.5 + 3.5 * x_mv + c(-0.1, 0.1, -0.05, 0.05, -0.08, 0.08)
y2_ctrl <- 10.0 + 0.5 * x_mv + c(0.05, -0.05, 0.02, -0.02, 0.04, -0.04)
y2_trt  <- 15.0 - 1.2 * x_mv + c(-0.05, 0.05, -0.02, 0.02, -0.04, 0.04)
for (nm in c("y1_ctrl", "y1_trt", "y2_ctrl", "y2_trt")) {
  yv <- get(nm)
  fit <- lm(yv ~ x_mv)
  cat(sprintf("%s: Intercept=%.12f Slope=%.12f\n", nm, coef(fit)[1], coef(fit)[2]))
}
