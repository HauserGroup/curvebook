# Regenerate the smoothing references in d1_curve_transforms.rs with R 4.5.2.
# Run with Rscript --vanilla. The Rust tests use the stored results offline.

emit <- function(label, values) {
  cat(label, "\n", sep = "")
  cat(paste(sprintf("%.17g", values), collapse = ", "), "\n", sep = "")
}

x <- c(0, 1, 3, 4, 7, 9, 10, 14, 17, 18, 23)
y <- c(1, 3, 0, 5, 2, 10, 4, 13, 6, 15, 11)
for (width in c(5L, 7L, 9L)) {
  fitted <- vapply(seq_along(x), function(i) {
    start <- min(max(1L, i - width %/% 2L), length(x) - width + 1L)
    rows <- seq.int(from = start, length.out = width)
    # Fit local degree-three polynomials using R's least-squares routine.
    offsets <- x[rows] - x[i]
    coefficients <- stats::lm.fit(outer(offsets, 0:3, `^`), y[rows])$coefficients
    coefficients[1L]
  }, numeric(1))
  emit(paste("Savitzky-Golay width", width), fitted)
}

x <- c(0, 1, 2, 4, 5, 7, 8, 11, 12, 14, 15, 17, 20)
y <- c(2, 4, 3, 6, 5, 8, 42, 10, 9, 13, 12, 13, 17)
# 13 * 0.65 = 8.45, so either truncation or nearest-integer rounding uses eight
# neighbors. delta = 0 requests a direct fit at each x rather than interpolation.
for (iterations in c(0L, 2L)) {
  emit(paste("LOWESS robustness iterations", iterations),
       stats::lowess(x, y, f = 0.65, iter = iterations, delta = 0)$y)
}
