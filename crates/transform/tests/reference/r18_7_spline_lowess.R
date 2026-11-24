# Reference oracle for CurveBook R18.7 FitSplineLowess (`crates/transform/tests/d1_r18_7_spline_lowess.rs`).
# Run with: Rscript crates/transform/tests/reference/r18_7_spline_lowess.R
# Verified on R 4.5.2 (2025-10-31).

options(digits = 16)

cat("=== 1. Natural Cubic Spline (de Boor 1978; stats::splinefun method='natural') ===\n")
x_nat <- c(0.0, 1.0, 2.5, 4.0, 6.0, 9.0)
y_nat <- c(1.2, 3.8, 2.1, 5.5, 4.9, 8.2)
sf <- stats::splinefun(x_nat, y_nat, method = "natural")
xq_nat <- c(-0.5, 0.0, 0.5, 1.75, 3.25, 5.0, 7.5, 9.0, 10.0)
yq_nat <- sf(xq_nat)
for (i in seq_along(xq_nat)) {
  cat(sprintf("nat[%d] x=%.4f y=%.15f\n", i, xq_nat[i], yq_nat[i]))
}

cat("\n=== 2. Smoothing Spline (Reinsch 1967; stats::smooth.spline all.knots=TRUE) ===\n")
x_sm <- c(0.0, 0.5, 1.2, 2.0, 2.8, 3.5, 4.4, 5.1, 6.0, 7.2, 8.5, 10.0)
y_sm <- c(0.4, 1.9, 3.1, 3.8, 4.9, 4.2, 3.6, 4.1, 5.3, 6.8, 7.4, 8.9)
ss5 <- stats::smooth.spline(x_sm, y_sm, df = 5.0, all.knots = TRUE)
cat(sprintf("ss5 df=%.15f\n", ss5$df))
xq_sm <- c(0.0, 1.0, 2.5, 4.0, 6.5, 9.0, 10.0)
yq_ss5 <- stats::predict(ss5, xq_sm)$y
for (i in seq_along(xq_sm)) {
  cat(sprintf("ss5[%d] x=%.4f y=%.15f\n", i, xq_sm[i], yq_ss5[i]))
}

ss8 <- stats::smooth.spline(x_sm, y_sm, df = 8.0, all.knots = TRUE)
cat(sprintf("ss8 df=%.15f\n", ss8$df))
yq_ss8 <- stats::predict(ss8, xq_sm)$y
for (i in seq_along(xq_sm)) {
  cat(sprintf("ss8[%d] x=%.4f y=%.15f\n", i, xq_sm[i], yq_ss8[i]))
}

cat("\n=== 3. LOWESS (Cleveland 1979; stats::lowess f=0.5, iter=3, delta=0) ===\n")
x_lo <- seq(1.0, 20.0, by = 1.0)
y_lo <- c(
  1.1, 2.0, 2.9, 4.2, 4.8,
  6.1, 6.9, 8.2, 8.8, 25.0,
  11.1, 11.9, 13.2, 13.8, 15.1,
  16.0, 17.2, 17.9, 19.1, 20.0
)
lo <- stats::lowess(x_lo, y_lo, f = 0.5, iter = 3, delta = 0.0)
for (i in seq_along(lo$x)) {
  cat(sprintf("lowess[%d] x=%.1f y=%.15f\n", i, lo$x[i], lo$y[i]))
}
