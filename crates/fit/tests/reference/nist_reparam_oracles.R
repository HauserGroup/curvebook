# Reference generator for `d1_nist_reparam_builtins.rs`.
#
# Run under R 4.5.2 (2025-10-31):
#   Rscript crates/fit/tests/reference/nist_reparam_oracles.R corpus/nist-strd
#
# Verifies algebraic reparameterizations of NIST StRD certified NLS and LLS
# benchmarks onto CurveBook's user-facing built-in equation forms, and emits
# secondary R `stats::nls` / `stats::lm` SEs for covariance-coupled parameters.

args <- commandArgs(TRUE)
root <- if (length(args) >= 1) args[1] else "corpus/nist-strd"

read_nls <- function(name) {
  path <- file.path(root, "nls", paste0(name, ".dat"))
  lines <- readLines(path)
  i <- tail(grep("^Data: +y", lines), 1)
  d <- read.table(text = lines[(i + 1):length(lines)], col.names = c("y", "x"))
  cl <- grep("^ +b[0-9]+ += ", lines, value = TRUE)
  cert <- t(sapply(strsplit(trimws(sub("^ +b[0-9]+ += ", "", cl)), " +"), function(v) as.numeric(tail(v, 2))))
  rss <- as.numeric(sub(".*: *", "", grep("Residual Sum of Squares", lines, value = TRUE)))
  list(d = d, est = cert[, 1], se = cert[, 2], rss = rss)
}

run_nls <- function(...) {
  r <- try(stats::nls(...), silent = TRUE)
  if (inherits(r, "try-error")) stats::nls(..., algorithm = "port") else r
}

ctl <- nls.control(maxiter = 2000, tol = 1e-8, minFactor = 1e-12, scaleOffset = 1)

emit <- function(label, fit, cert_est, cert_se, cert_rss) {
  co <- summary(fit)$coefficients
  cat(sprintf("=== %s (R RSS=%.15e, cert RSS=%.15e, rel=%.2e) ===\n",
              label, deviance(fit), cert_rss, abs(deviance(fit) / cert_rss - 1)))
  for (p in names(cert_est)) {
    r_est <- co[p, 1]
    r_se <- co[p, 2]
    ce <- cert_est[[p]]
    cs <- cert_se[[p]]
    se_note <- if (is.na(cs)) {
      sprintf("R_se=%.15e (vcov)", r_se)
    } else {
      sprintf("cert_se=%.15e R_se=%.15e rel=%.2e", cs, r_se, abs(r_se / cs - 1))
    }
    cat(sprintf("  %-12s cert_est=%.15e R_est=%.15e rel=%.2e | %s\n",
                p, ce, r_est, abs(r_est / ce - 1), se_note))
  }
}

# 1. Misra1d -> michaelis_menten & one_site_specific_binding
m <- read_nls("Misra1d"); b <- m$est; s <- m$se
f <- run_nls(y ~ Vmax * x / (Km + x), m$d, start = list(Vmax = 450, Km = 3000), control = ctl)
emit("Misra1d>MM", f, c(Vmax = b[1], Km = 1 / b[2]), c(Vmax = s[1], Km = s[2] / b[2]^2), m$rss)

# 2. BoxBOD -> one_phase_association / exponential_plateau (Y0=0) & one_phase_decay (y_rev = b1 - y? No: direct fit with Y0=0, Plateau free)
m <- read_nls("BoxBOD"); b <- m$est; s <- m$se
f <- run_nls(y ~ Plateau * (1 - exp(-K * x)), m$d, start = list(Plateau = 200, K = 0.5), control = ctl)
emit("BoxBOD>OPA", f, c(Plateau = b[1], K = b[2]), c(Plateau = s[1], K = s[2]), m$rss)

# 3. Misra1a -> one_phase_association / exponential_plateau / one_phase_decay (Y0=0)
m <- read_nls("Misra1a"); b <- m$est; s <- m$se
f <- run_nls(y ~ Plateau * (1 - exp(-K * x)), m$d, start = list(Plateau = 250, K = 5e-4), control = ctl)
emit("Misra1a>OPA", f, c(Plateau = b[1], K = b[2]), c(Plateau = s[1], K = s[2]), m$rss)

# 4. DanWood -> power_law (a * x^b)
m <- read_nls("DanWood"); b <- m$est; s <- m$se
f <- run_nls(y ~ a * x^bb, m$d, start = list(a = 1, bb = 5), control = ctl)
emit("DanWood>pow", f, c(a = b[1], bb = b[2]), c(a = s[1], bb = s[2]), m$rss)

# 5. Eckerle4 -> gaussian (Amplitude * exp(-0.5 * ((x - Mean) / SD)^2))
m <- read_nls("Eckerle4"); b <- m$est; s <- m$se
f <- run_nls(y ~ Amplitude * exp(-0.5 * ((x - Mean) / SD)^2), m$d,
             start = list(Amplitude = 0.3, Mean = 451, SD = 5), control = ctl)
emit("Eckerle4>Gaussian", f,
     c(Amplitude = b[1] / b[2], Mean = b[3], SD = b[2]),
     c(Amplitude = NA, Mean = s[3], SD = s[2]), m$rss)

# 6. Rat42 -> logistic_growth, log_agonist_vs_response_4pl (Bottom=0), log_inhibitor_vs_response_4pl (Bottom=0, -x)
m <- read_nls("Rat42"); b <- m$est; s <- m$se
f <- run_nls(y ~ YM * Y0 / ((YM - Y0) * exp(-K * x) + Y0), m$d,
             start = as.list(1.05 * c(YM = b[1], Y0 = b[1] / (1 + exp(b[2])), K = b[3])), control = ctl)
emit("Rat42>LogisticGrowth", f,
     c(YM = b[1], Y0 = b[1] / (1 + exp(b[2])), K = b[3]),
     c(YM = s[1], Y0 = NA, K = s[3]), m$rss)

f <- run_nls(y ~ Top / (1 + 10^((LogEC50 - x) * HillSlope)), m$d,
             start = as.list(1.05 * c(Top = b[1], LogEC50 = b[2] / b[3], HillSlope = b[3] / log(10))), control = ctl)
emit("Rat42>4PL_Agonist", f,
     c(Top = b[1], LogEC50 = b[2] / b[3], HillSlope = b[3] / log(10)),
     c(Top = s[1], LogEC50 = NA, HillSlope = s[3] / log(10)), m$rss)

# 7. Rat43 -> log_agonist_vs_response_5pl (Bottom=0)
m <- read_nls("Rat43"); b <- m$est; s <- m$se
f <- run_nls(y ~ Top / (1 + 10^((LogXb - x) * HillSlope))^S, m$d,
             start = as.list(1.05 * c(Top = b[1], LogXb = b[2] / b[3], HillSlope = b[3] / log(10), S = 1 / b[4])),
             control = ctl)
emit("Rat43>5PL", f,
     c(Top = b[1], LogXb = b[2] / b[3], HillSlope = b[3] / log(10), S = 1 / b[4]),
     c(Top = s[1], LogXb = NA, HillSlope = s[3] / log(10), S = s[4] / b[4]^2), m$rss)

# 8. Lanczos3 -> three_phase_decay (Plateau=0)
m <- read_nls("Lanczos3"); b <- m$est; s <- m$se
y0 <- b[1] + b[3] + b[5]
f <- run_nls(y ~ Y0 * (PF / 100 * exp(-KF * x) + PS / 100 * exp(-KS * x) + (1 - (PF + PS) / 100) * exp(-KV * x)),
             m$d,
             start = as.list(1.02 * c(Y0 = y0, PF = 100 * b[5] / y0, PS = 100 * b[3] / y0,
                                      KF = b[6], KS = b[4], KV = b[2])),
             control = ctl)
emit("Lanczos3>ThreePhaseDecay", f,
     c(Y0 = y0, PF = 100 * b[5] / y0, PS = 100 * b[3] / y0, KF = b[6], KS = b[4], KV = b[2]),
     c(Y0 = NA, PF = NA, PS = NA, KF = s[6], KS = s[4], KV = s[2]), m$rss)

# 9. Eckerle4 -> gaussian_area (Area = b1 * sqrt(2*pi), Mean = b3, SD = b2)
m <- read_nls("Eckerle4"); b <- m$est; s <- m$se
s2p <- sqrt(2 * pi)
f <- run_nls(y ~ (Area / (SD * sqrt(2 * pi))) * exp(-0.5 * ((x - Mean) / SD)^2), m$d,
             start = list(Area = b[1] * s2p * 1.05, Mean = 451, SD = 5), control = ctl)
emit("Eckerle4>GaussianArea", f,
     c(Area = b[1] * s2p, Mean = b[3], SD = b[2]),
     c(Area = s[1] * s2p, Mean = s[3], SD = s[2]), m$rss)

# 10. Rat42 -> boltzmann_sigmoid (Bottom=0, Top = b1, V50 = b2 / b3, Slope = 1 / b3)
m <- read_nls("Rat42"); b <- m$est; s <- m$se
f <- run_nls(y ~ Top / (1 + exp((V50 - x) / Slope)), m$d,
             start = as.list(1.05 * c(Top = b[1], V50 = b[2] / b[3], Slope = 1 / b[3])), control = ctl)
emit("Rat42>BoltzmannSigmoid", f,
     c(Top = b[1], V50 = b[2] / b[3], Slope = 1 / b[3]),
     c(Top = s[1], V50 = NA, Slope = s[3] / b[3]^2), m$rss)

# 11. BoxBOD -> two_phase_association (Y0=0, PercentFast=100, KSlow=0.01)
m <- read_nls("BoxBOD"); b <- m$est; s <- m$se
f <- run_nls(y ~ Plateau * (1 - exp(-KFast * x)), m$d,
             start = list(Plateau = 200, KFast = 0.5), control = ctl)
emit("BoxBOD>TwoPhaseAssociation", f,
     c(Plateau = b[1], KFast = b[2]),
     c(Plateau = s[1], KFast = s[2]), m$rss)

# 12. DanWood -> power_series_two_term (C=0, D=1 => y = A * x^B)
m <- read_nls("DanWood"); b <- m$est; s <- m$se
f <- run_nls(y ~ A * x^B, m$d, start = list(A = 1, B = 5), control = ctl)
emit("DanWood>PowerSeriesTwoTerm", f,
     c(A = b[1], B = b[2]),
     c(A = s[1], B = s[2]), m$rss)
