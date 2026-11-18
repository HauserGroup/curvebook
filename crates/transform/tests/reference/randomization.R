# Independent R 4.5.2 corroboration script for Binomial and Poisson CDFs in d1_randomization.rs.
# Run with: Rscript --vanilla randomization.R

emit_binom <- function(label, n, p, cutoffs) {
  vals <- stats::pbinom(cutoffs, size = n, prob = p)
  cat(sprintf("[%s] n=%d p=%.17g\n", label, n, p))
  for (i in seq_along(cutoffs)) {
    cat(sprintf("  k=%3d: cdf=%.17g\n", cutoffs[i], vals[i]))
  }
}

emit_pois <- function(label, lambda, cutoffs) {
  vals <- stats::ppois(cutoffs, lambda = lambda)
  cat(sprintf("[%s] lambda=%.17g\n", label, lambda))
  for (i in seq_along(cutoffs)) {
    cat(sprintf("  k=%3d: cdf=%.17g\n", cutoffs[i], vals[i]))
  }
}

emit_binom("Binom32_Bernoulli", 32L, 0.375, c(8L, 10L, 12L, 15L))
emit_binom("Binom33_Btrs", 33L, 0.375, c(8L, 11L, 12L, 16L))
emit_binom("Binom39_Geo", 39L, 0.25, c(6L, 8L, 10L, 13L))
emit_binom("Binom40_Btrs", 40L, 0.25, c(6L, 9L, 10L, 14L))
emit_binom("Binom2000_Geo", 2000L, 0.004, c(4L, 7L, 8L, 12L))
emit_binom("Binom60_FlipBtrs", 60L, 0.75, c(40L, 44L, 45L, 49L))
emit_binom("Binom64_FlipGeo", 64L, 0.875, c(52L, 55L, 56L, 59L))

emit_pois("Pois4p5_Small", 4.5, c(1L, 3L, 4L, 7L))
emit_pois("Pois29p5_Below30", 29.5, c(22L, 27L, 29L, 37L))
emit_pois("Pois30_At30", 30.0, c(22L, 27L, 30L, 37L))
emit_pois("Pois120_Large", 120.0, c(105L, 115L, 120L, 134L))
