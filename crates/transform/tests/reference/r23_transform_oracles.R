# Independent R 4.5.2 (2025-10-31) black-box reference generator for CurveBook
# R23.1 & R23.3 (`crates/transform`):
#   1. Row Statistics (Totals, Mean/SD/SEM/%CV/CI, Median/Q1/Q3/IQR Type-6 &
#      Type-7 custom percentiles, Geometric Mean/GeoSD/CI)
#   2. Fraction of Total (row/column/grand + exact Clopper-Pearson & Wilson CIs)
#   3. Curve Calculus (1st/2nd shifted Lagrange derivatives on uniform &
#      non-uniform grids, Savitzky-Golay smoothing & derivatives, cumulative
#      trapezoidal integral with zero and nonzero baselines)
#   4. Baseline Removal variants (ColumnsRowWise, ColumnsMean, Constant,
#      RowRangeMean, FirstAndLastLinear, OlsWithX, Paired; all operations)
#   5. Special Functions (gamma, lgamma, igamma/igammac, beta, ibeta, digamma,
#      Bessel J/Y/I/K, plus integral cross-checks for hyp1f1, hyp2f1, U)
#
# Run under R 4.5.2:
#   /usr/local/bin/Rscript crates/transform/tests/reference/r23_transform_oracles.R
#
# Companion Python high-precision verifier (arbitrary-precision Decimal for
# hyp1f1/hyp2f1/confluent_u and R23.1 formula/window edge cases):
#   /opt/homebrew/bin/python3 crates/transform/tests/reference/r23_transform_oracles.py

if (getRversion() != "4.5.2") {
  stop(sprintf("Expected R 4.5.2, found %s", as.character(getRversion())))
}

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
# 1. Row Statistics (Grouped/XY replicate rows; positive & mixed-count cases)
# ============================================================================
# Four rows of replicates; row 3 has a missing replicate (n = 4) and row 4 has
# a zero value to verify geometric statistics return null when any y <= 0.
row_stats_inputs <- list(
  row0 = c(10.2, 12.5, 11.8, 13.1, 10.9),
  row1 = c(2.4, 4.8, 9.6, 6.2, 3.5),
  row2_missing = c(100.5, 104.2, 98.7, 101.6),
  row3_with_zero = c(0.0, 2.5, 5.0, 7.5, 10.0)
)

compute_row_stats <- function(v, conf = 0.95) {
  n <- length(v)
  tot <- sum(v)
  m <- mean(v)
  s <- sd(v)
  sem <- s / sqrt(n)
  cv_pct <- if (m != 0) 100.0 * s / abs(m) else NA_real_
  t_crit <- qt(1 - (1 - conf) / 2, df = n - 1)
  ci_lo <- m - t_crit * sem
  ci_hi <- m + t_crit * sem
  q6 <- unname(quantile(v, probs = c(0.25, 0.50, 0.75), type = 6))
  q7_custom <- unname(quantile(v, probs = c(0.10, 0.90), type = 7))
  if (all(v > 0)) {
    lv <- log(v)
    ml <- mean(lv)
    sl <- sd(lv)
    seml <- sl / sqrt(n)
    gmean <- exp(ml)
    gsd <- exp(sl)
    gci_lo <- exp(ml - t_crit * seml)
    gci_hi <- exp(ml + t_crit * seml)
  } else {
    gmean <- NA_real_; gsd <- NA_real_; gci_lo <- NA_real_; gci_hi <- NA_real_
  }
  list(
    values = as_arr(v),
    n = as.integer(n),
    total = tot,
    mean = m,
    sd = s,
    sem = sem,
    cv_percent = cv_pct,
    mean_ci95_lo = ci_lo,
    mean_ci95_hi = ci_hi,
    min = min(v),
    q1_type6 = q6[1],
    median = q6[2],
    q3_type6 = q6[3],
    iqr_type6 = q6[3] - q6[1],
    max = max(v),
    range = max(v) - min(v),
    p10_type7 = q7_custom[1],
    p90_type7 = q7_custom[2],
    geo_mean = gmean,
    geo_sd_factor = gsd,
    geo_ci95_lo = gci_lo,
    geo_ci95_hi = gci_hi
  )
}

row_stats_out <- lapply(row_stats_inputs, compute_row_stats)

# ============================================================================
# 2. Fraction of Total (3x4 count matrix + exact Clopper-Pearson & Wilson CIs)
# ============================================================================
# Matrix stored row-major: 3 rows x 4 columns (includes zero cell at [3,1]).
fot_mat <- matrix(
  c(12, 18,  6, 24,
     5, 15, 20, 10,
     0,  9, 27, 14),
  nrow = 3, ncol = 4, byrow = TRUE
)

clopper_pearson_ci <- function(k, n, conf = 0.95) {
  bt <- stats::binom.test(as.integer(k), as.integer(n), conf.level = conf)
  unname(bt$conf.int)
}

wilson_ci <- function(k, n, conf = 0.95) {
  # Wilson (1927) score interval without continuity correction
  p <- k / n
  z <- qnorm(1 - (1 - conf) / 2)
  z2 <- z * z
  denom <- 1 + z2 / n
  center <- (p + z2 / (2 * n)) / denom
  half <- (z / denom) * sqrt(p * (1 - p) / n + z2 / (4 * n * n))
  lo <- if (k == 0) 0.0 else max(0.0, center - half)
  hi <- if (k == n) 1.0 else min(1.0, center + half)
  c(lo, hi)
}

wilson_cc_ci <- function(k, n, conf = 0.95) {
  # Continuity-corrected Wilson score interval via stats::prop.test
  unname(suppressWarnings(stats::prop.test(k, n, conf.level = conf, correct = TRUE)$conf.int))
}

row_sums <- rowSums(fot_mat)
col_sums <- colSums(fot_mat)
grand_sum <- sum(fot_mat)

fot_cells <- list()
for (r in seq_len(nrow(fot_mat))) {
  for (c_idx in seq_len(ncol(fot_mat))) {
    k <- fot_mat[r, c_idx]
    cp_row <- clopper_pearson_ci(k, row_sums[r])
    w_row <- wilson_ci(k, row_sums[r])
    wcc_row <- wilson_cc_ci(k, row_sums[r])
    cp_col <- clopper_pearson_ci(k, col_sums[c_idx])
    w_col <- wilson_ci(k, col_sums[c_idx])
    cp_grand <- clopper_pearson_ci(k, grand_sum)
    w_grand <- wilson_ci(k, grand_sum)
    fot_cells[[sprintf("r%d_c%d", r - 1L, c_idx - 1L)]] <- list(
      count = k,
      row_fraction = k / row_sums[r],
      row_percent = 100.0 * k / row_sums[r],
      col_fraction = k / col_sums[c_idx],
      col_percent = 100.0 * k / col_sums[c_idx],
      grand_fraction = k / grand_sum,
      grand_percent = 100.0 * k / grand_sum,
      row_clopper_pearson_95 = as_arr(cp_row),
      row_wilson_95 = as_arr(w_row),
      row_wilson_cc_95 = as_arr(wcc_row),
      col_clopper_pearson_95 = as_arr(cp_col),
      col_wilson_95 = as_arr(w_col),
      grand_clopper_pearson_95 = as_arr(cp_grand),
      grand_wilson_95 = as_arr(w_grand)
    )
  }
}

# Boundary CI case: k = n (100% proportion)
fot_boundary_full <- list(
  k = 15L, n = 15L,
  clopper_pearson_95 = as_arr(clopper_pearson_ci(15, 15)),
  wilson_95 = as_arr(wilson_ci(15, 15)),
  wilson_cc_95 = as_arr(wilson_cc_ci(15, 15))
)

fraction_of_total_out <- list(
  matrix_row_major = lapply(seq_len(nrow(fot_mat)), function(r) as_arr(as.numeric(fot_mat[r, ]))),
  row_totals = as_arr(as.numeric(row_sums)),
  col_totals = as_arr(as.numeric(col_sums)),
  grand_total = as.numeric(grand_sum),
  cells = fot_cells,
  boundary_k_eq_n = fot_boundary_full
)

# ============================================================================
# 3. Curve Calculus (1st/2nd Derivatives, Savitzky-Golay, Cumulative Integral)
# ============================================================================
# 3a. Non-uniform grid polynomial + sin test curve: y = 0.5*x^3 - 2*x^2 + 3*x + 1
x_nonunif <- c(0.0, 0.4, 1.0, 1.8, 2.5, 3.6, 4.5, 6.0)
y_nonunif <- 0.5 * x_nonunif^3 - 2.0 * x_nonunif^2 + 3.0 * x_nonunif + 1.0

# Exact 3-point Lagrange quadratic divided differences (matches curve.rs)
lagrange_3pt_deriv <- function(x, y, order = 1L) {
  n <- length(x)
  out <- numeric(n)
  for (i in seq_len(n)) {
    idx <- if (i == 1L) c(1L, 2L, 3L) else if (i == n) c(n - 2L, n - 1L, n) else c(i - 1L, i, i + 1L)
    x0 <- x[idx[1]]; x1 <- x[idx[2]]; x2 <- x[idx[3]]
    y0 <- y[idx[1]]; y1 <- y[idx[2]]; y2 <- y[idx[3]]
    h1 <- x1 - x0; h2 <- x2 - x1
    s1 <- (y1 - y0) / h1
    s2 <- (y2 - y1) / h2
    r1 <- h1 / (h1 + h2)
    r2 <- h2 / (h1 + h2)
    if (order == 1L) {
      out[i] <- if (i == 1L) s1 - r1 * (s2 - s1) else if (i == n) s2 + r2 * (s2 - s1) else r2 * s1 + r1 * s2
    } else {
      out[i] <- 2.0 * (s2 - s1) / (h1 + h2)
    }
  }
  out
}

# Cumulative trapezoidal integral relative to baseline
cum_trapz <- function(x, y, baseline = 0.0) {
  n <- length(x)
  out <- numeric(n)
  out[1] <- 0.0
  for (i in 2:n) {
    out[i] <- out[i - 1L] + 0.5 * ((y[i - 1L] - baseline) + (y[i] - baseline)) * (x[i] - x[i - 1L])
  }
  out
}

# 3b. Uniform grid noisy signal for Savitzky-Golay (smooth + 1st/2nd deriv)
x_unif <- seq(0.0, 10.0, by = 1.0)
y_unif <- c(1.05, 2.32, 4.88, 8.71, 13.40, 17.65, 20.12, 19.85, 16.40, 10.95, 5.10)

# Local polynomial least-squares Savitzky-Golay (Gorry 1990 boundary handling)
sg_local_poly <- function(x, y, width, poly_order = 3L, deriv = 0L) {
  n <- length(x)
  half <- as.integer((width - 1L) / 2L)
  out <- numeric(n)
  for (i in seq_len(n)) {
    start <- if (i <= half) 1L else if (i + half > n) n - width + 1L else i - half
    end <- start + width - 1L
    dx <- x[start:end] - x[i]
    Xmat <- outer(dx, 0:poly_order, "^")
    coefs <- unname(lm.fit(Xmat, y[start:end])$coefficients)
    out[i] <- factorial(deriv) * coefs[deriv + 1L]
  }
  out
}

curve_calculus_out <- list(
  nonuniform = list(
    x = as_arr(x_nonunif),
    y = as_arr(y_nonunif),
    d1_lagrange3 = as_arr(lagrange_3pt_deriv(x_nonunif, y_nonunif, 1L)),
    d2_lagrange3 = as_arr(lagrange_3pt_deriv(x_nonunif, y_nonunif, 2L)),
    integral_baseline_0 = as_arr(cum_trapz(x_nonunif, y_nonunif, 0.0)),
    integral_baseline_1_5 = as_arr(cum_trapz(x_nonunif, y_nonunif, 1.5))
  ),
  uniform = list(
    x = as_arr(x_unif),
    y = as_arr(y_unif),
    d1_lagrange3 = as_arr(lagrange_3pt_deriv(x_unif, y_unif, 1L)),
    d2_lagrange3 = as_arr(lagrange_3pt_deriv(x_unif, y_unif, 2L)),
    sg_w5_p3_smooth = as_arr(sg_local_poly(x_unif, y_unif, 5L, 3L, 0L)),
    sg_w7_p3_smooth = as_arr(sg_local_poly(x_unif, y_unif, 7L, 3L, 0L)),
    sg_w9_p3_smooth = as_arr(sg_local_poly(x_unif, y_unif, 9L, 3L, 0L)),
    sg_w5_p3_d1 = as_arr(sg_local_poly(x_unif, y_unif, 5L, 3L, 1L)),
    sg_w5_p3_d2 = as_arr(sg_local_poly(x_unif, y_unif, 5L, 3L, 2L)),
    sg_w7_p2_d1 = as_arr(sg_local_poly(x_unif, y_unif, 7L, 2L, 1L)),
    sg_w7_p2_d2 = as_arr(sg_local_poly(x_unif, y_unif, 7L, 2L, 2L)),
    integral_baseline_0 = as_arr(cum_trapz(x_unif, y_unif, 0.0))
  )
)

# ============================================================================
# 4. Baseline Removal Variants
# ============================================================================
# 8-row XY dataset with 4 columns:
#   col0 = Blank1, col1 = SampleA, col2 = Blank2, col3 = SampleB
bl_x <- c(0.0, 1.0, 2.5, 4.0, 6.0, 8.0, 10.0, 12.0)
bl_c0 <- c(2.0, 2.2, 2.5, 2.8, 3.2, 3.6, 4.0, 4.4)
bl_c1 <- c(2.1, 4.5, 9.2, 14.0, 16.5, 11.2, 5.1, 4.6)
bl_c2 <- c(1.0, 1.1, 1.3, 1.5, 1.7, 1.9, 2.1, 2.3)
bl_c3 <- c(1.2, 3.0, 6.4, 9.8, 11.5, 8.4, 3.5, 2.5)

apply_bl_ops <- function(y, b) {
  list(
    baseline = as_arr(b),
    subtract = as_arr(y - b),
    divide = as_arr(y / b),
    percent = as_arr(100.0 * y / b),
    fractional_change = as_arr((y - b) / b),
    percent_change = as_arr(100.0 * (y - b) / b),
    add = as_arr(y + b),
    multiply = as_arr(y * b)
  )
}

# ColumnsRowWise using col0 as baseline for SampleA (col1)
bl_rowwise <- apply_bl_ops(bl_c1, bl_c0)
# ColumnsMean: single grand mean of col0 & col2 across all rows
grand_bl_mean <- mean(c(bl_c0, bl_c2))
bl_cols_mean <- apply_bl_ops(bl_c1, rep(grand_bl_mean, length(bl_c1)))
# Constant(2.5)
bl_constant <- apply_bl_ops(bl_c1, rep(2.5, length(bl_c1)))
# RowRangeMean: inclusive rows 0..=1 (1-based 1:2) of SampleA
rrm_val <- mean(bl_c1[1:2])
bl_row_range_mean <- apply_bl_ops(bl_c1, rep(rrm_val, length(bl_c1)))
# FirstAndLastLinear (first_k = 2, last_k = 2) on row index (0..7)
# Centroids: x_start = mean(0, 1) = 0.5, y_start = mean(bl_c1[1:2])
#            x_end   = mean(6, 7) = 6.5, y_end   = mean(bl_c1[7:8])
r_idx <- 0:7
x_start_r <- mean(r_idx[1:2]); y_start_r <- mean(bl_c1[1:2])
x_end_r <- mean(r_idx[7:8]);   y_end_r <- mean(bl_c1[7:8])
slope_fl_row <- (y_end_r - y_start_r) / (x_end_r - x_start_r)
b_fl_row <- y_start_r + slope_fl_row * (r_idx - x_start_r)
# FirstAndLastLinear using X coordinate centroids
x_start_x <- mean(bl_x[1:2]); x_end_x <- mean(bl_x[7:8])
slope_fl_x <- (y_end_r - y_start_r) / (x_end_x - x_start_x)
b_fl_x <- y_start_r + slope_fl_x * (bl_x - x_start_x)
# OLS-with-X over first 2 and last 2 baseline rows (rows 1,2,7,8 in 1-based)
ols_idx <- c(1L, 2L, 7L, 8L)
ols_fit <- lm(bl_c1[ols_idx] ~ bl_x[ols_idx])
b_ols_x <- unname(coef(ols_fit)[1] + coef(ols_fit)[2] * bl_x)
# Paired every-other-column: (col0 -> col1) and (col2 -> col3)
bl_paired_pair0 <- apply_bl_ops(bl_c1, bl_c0)
bl_paired_pair1 <- apply_bl_ops(bl_c3, bl_c2)

baseline_removal_out <- list(
  x = as_arr(bl_x),
  col0_blank1 = as_arr(bl_c0),
  col1_sampleA = as_arr(bl_c1),
  col2_blank2 = as_arr(bl_c2),
  col3_sampleB = as_arr(bl_c3),
  columns_row_wise_col1_vs_col0 = bl_rowwise,
  columns_mean_col0_col2 = list(grand_baseline_mean = grand_bl_mean, results = bl_cols_mean),
  constant_2_5 = bl_constant,
  row_range_mean_rows_0_to_1 = list(baseline_value = rrm_val, results = bl_row_range_mean),
  first_and_last_linear_row_index = list(
    first_k = 2L, last_k = 2L,
    centroid_start = as_arr(c(x_start_r, y_start_r)),
    centroid_end = as_arr(c(x_end_r, y_end_r)),
    results = apply_bl_ops(bl_c1, b_fl_row)
  ),
  first_and_last_linear_x_coord = list(
    first_k = 2L, last_k = 2L,
    centroid_start = as_arr(c(x_start_x, y_start_r)),
    centroid_end = as_arr(c(x_end_x, y_end_r)),
    results = apply_bl_ops(bl_c1, b_fl_x)
  ),
  ols_with_x_first2_last2 = list(
    intercept = unname(coef(ols_fit)[1]),
    slope = unname(coef(ols_fit)[2]),
    results = apply_bl_ops(bl_c1, b_ols_x)
  ),
  paired_every_other_column = list(
    col1_minus_col0 = bl_paired_pair0,
    col3_minus_col2 = bl_paired_pair1
  )
)

# ============================================================================
# 5. Special Functions (R 4.5.2 reference + integral checks for 1F1, 2F1, U)
# ============================================================================
hyp1f1_integral <- function(a, b, z) {
  # DLMF 13.4.1: M(a,b,z) = Gamma(b)/(Gamma(a)*Gamma(b-a)) * int_0^1 e^(zt) t^(a-1) (1-t)^(b-a-1) dt
  # Valid for Re(b) > Re(a) > 0
  pref <- exp(lgamma(b) - lgamma(a) - lgamma(b - a))
  val <- integrate(function(t) exp(z * t) * t^(a - 1) * (1 - t)^(b - a - 1),
                   0, 1, rel.tol = 1e-13, abs.tol = 1e-15)$value
  pref * val
}

hyp2f1_integral <- function(a, b, c_p, z) {
  # DLMF 15.6.1: 2F1(a,b;c;z) = Gamma(c)/(Gamma(b)*Gamma(c-b)) * int_0^1 t^(b-1) (1-t)^(c-b-1) (1-zt)^(-a) dt
  # Valid for Re(c) > Re(b) > 0 and z < 1
  pref <- exp(lgamma(c_p) - lgamma(b) - lgamma(c_p - b))
  val <- integrate(function(t) t^(b - 1) * (1 - t)^(c_p - b - 1) * (1 - z * t)^(-a),
                   0, 1, rel.tol = 1e-13, abs.tol = 1e-15)$value
  pref * val
}

confluent_u_integral <- function(a, b, z) {
  # DLMF 13.4.4 with exact substitution t = u^(1/a) removing any t = 0 singularity:
  # U(a,b,z) = (1/Gamma(a+1)) * int_0^inf exp(-z*u^(1/a)) * (1 + u^(1/a))^(b-a-1) du
  val <- integrate(function(u) {
    t <- u^(1 / a)
    exp(-z * t + (b - a - 1) * log1p(t))
  }, 0, Inf, rel.tol = 1e-12, abs.tol = 1e-14)$value
  val / gamma(a + 1)
}

special_functions_out <- list(
  gamma = list(
    "gamma(0.5)" = gamma(0.5),
    "gamma(3.5)" = gamma(3.5),
    "gamma(6.0)" = gamma(6.0),
    "gamma(-1.5)" = gamma(-1.5)
  ),
  ln_gamma = list(
    "lgamma(0.5)" = lgamma(0.5),
    "lgamma(3.5)" = lgamma(3.5),
    "lgamma(50.0)" = lgamma(50.0),
    "lgamma(1000.0)" = lgamma(1000.0)
  ),
  igamma_lower_P = list(
    "igamma(0.5, 1.0)" = pgamma(1.0, shape = 0.5, rate = 1.0),
    "igamma(2.5, 1.8)" = pgamma(1.8, shape = 2.5, rate = 1.0),
    "igamma(5.0, 3.0)" = pgamma(3.0, shape = 5.0, rate = 1.0),
    "igamma(10.0, 15.0)" = pgamma(15.0, shape = 10.0, rate = 1.0),
    "igamma(50.0, 60.0)" = pgamma(60.0, shape = 50.0, rate = 1.0)
  ),
  igammac_upper_Q = list(
    "igammac(0.5, 1.0)" = pgamma(1.0, shape = 0.5, rate = 1.0, lower.tail = FALSE),
    "igammac(2.5, 1.8)" = pgamma(1.8, shape = 2.5, rate = 1.0, lower.tail = FALSE),
    "igammac(5.0, 3.0)" = pgamma(3.0, shape = 5.0, rate = 1.0, lower.tail = FALSE),
    "igammac(10.0, 15.0)" = pgamma(15.0, shape = 10.0, rate = 1.0, lower.tail = FALSE),
    "igammac(50.0, 60.0)" = pgamma(60.0, shape = 50.0, rate = 1.0, lower.tail = FALSE)
  ),
  beta = list(
    "beta(0.5, 0.5)" = beta(0.5, 0.5),
    "beta(2.5, 3.5)" = beta(2.5, 3.5),
    "beta(10.0, 5.0)" = beta(10.0, 5.0)
  ),
  ibeta = list(
    "ibeta(0.5, 0.5, 0.25)" = pbeta(0.25, 0.5, 0.5),
    "ibeta(2.5, 3.5, 0.4)" = pbeta(0.4, 2.5, 3.5),
    "ibeta(12.0, 8.0, 0.72)" = pbeta(0.72, 12.0, 8.0),
    "ibeta(3.0, 7.0, 0.15)" = pbeta(0.15, 3.0, 7.0)
  ),
  digamma = list(
    "digamma(0.5)" = digamma(0.5),
    "digamma(1.0)" = digamma(1.0),
    "digamma(2.5)" = digamma(2.5),
    "digamma(10.0)" = digamma(10.0),
    "digamma(-1.5)" = digamma(-1.5)
  ),
  bessel_j = list(
    "J0(0.0)" = besselJ(0.0, 0),
    "J0(2.5)" = besselJ(2.5, 0),
    "J1(2.5)" = besselJ(2.5, 1),
    "J2(4.0)" = besselJ(4.0, 2),
    "J3(7.5)" = besselJ(7.5, 3),
    "J5(12.0)" = besselJ(12.0, 5),
    "J3(-2.5)" = -besselJ(2.5, 3),
    "J(-3, 2.5)" = -besselJ(2.5, 3)
  ),
  bessel_y = list(
    "Y0(1.5)" = besselY(1.5, 0),
    "Y1(1.5)" = besselY(1.5, 1),
    "Y2(4.0)" = besselY(4.0, 2),
    "Y3(7.5)" = besselY(7.5, 3),
    "Y5(12.0)" = besselY(12.0, 5),
    "Y(-3, 1.5)" = -besselY(1.5, 3)
  ),
  bessel_i = list(
    "I0(0.0)" = besselI(0.0, 0),
    "I0(2.5)" = besselI(2.5, 0),
    "I1(2.5)" = besselI(2.5, 1),
    "I3(2.5)" = besselI(2.5, 3),
    "I5(10.0)" = besselI(10.0, 5),
    "I3(-2.5)" = -besselI(2.5, 3),
    "I(-3, 2.5)" = besselI(2.5, 3)
  ),
  bessel_k = list(
    "K0(1.5)" = besselK(1.5, 0),
    "K1(1.5)" = besselK(1.5, 1),
    "K2(2.5)" = besselK(2.5, 2),
    "K3(1.5)" = besselK(1.5, 3),
    "K5(4.0)" = besselK(4.0, 5),
    "K(-3, 1.5)" = besselK(1.5, 3)
  ),
  hyp1f1_integral_crosscheck = list(
    "M(1.5, 3.0, 1.2)" = hyp1f1_integral(1.5, 3.0, 1.2),
    "M(0.5, 1.5, -2.0)" = hyp1f1_integral(0.5, 1.5, -2.0),
    "M(2.0, 5.0, 3.5)" = hyp1f1_integral(2.0, 5.0, 3.5),
    "M(1.25, 2.75, -0.8)" = hyp1f1_integral(1.25, 2.75, -0.8)
  ),
  hyp2f1_integral_crosscheck = list(
    "2F1(0.5, 1.0; 1.5; 0.25)" = hyp2f1_integral(0.5, 1.0, 1.5, 0.25),
    "2F1(1.5, 2.0; 3.5; 0.5)" = hyp2f1_integral(1.5, 2.0, 3.5, 0.5),
    "2F1(0.75, 1.25; 2.5; -0.6)" = hyp2f1_integral(0.75, 1.25, 2.5, -0.6),
    "2F1(2.0, 1.5; 4.0; -2.0)" = hyp2f1_integral(2.0, 1.5, 4.0, -2.0)
  ),
  confluent_u_integral_crosscheck = list(
    "U(1.5, 2.0, 1.0)" = confluent_u_integral(1.5, 2.0, 1.0),
    "U(0.5, 1.5, 2.5)" = confluent_u_integral(0.5, 1.5, 2.5),
    "U(2.0, 1.25, 3.0)" = confluent_u_integral(2.0, 1.25, 3.0),
    "U(1.25, 0.75, 0.5)" = confluent_u_integral(1.25, 0.75, 0.5)
  )
)

payload <- list(
  generator = "crates/transform/tests/reference/r23_transform_oracles.R",
  r_version = as.character(getRversion()),
  row_statistics = row_stats_out,
  fraction_of_total = fraction_of_total_out,
  curve_calculus = curve_calculus_out,
  baseline_removal = baseline_removal_out,
  special_functions_r = special_functions_out
)

out_path <- "crates/transform/tests/reference/r23_transform_oracles_r.json"
writeLines(to_json(payload), con = out_path)
cat(sprintf("Wrote %s\n", out_path))
