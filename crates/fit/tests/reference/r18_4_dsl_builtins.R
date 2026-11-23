# Reference oracle generator for `d1_r18_4_dsl_builtins_and_init_rules.rs`.
#
# Run under R 4.5.2 (2025-10-31):
#   Rscript crates/fit/tests/reference/r18_4_dsl_builtins.R
#
# Generates independent high-precision reference values for:
# - Regularized incomplete beta and gamma (`ibeta`, `igamma`, `igammac`)
# - Normal, Student t, Snedecor F, Chi-square, and Binomial distributions
# - Algebraic helpers (`fact`, `logit`, `probit`)
# - General integer-order modified Bessel functions (`besseli`, `besselk`)
# - Empirical OLS endpoint/midpoint slopes for `[InitialValues]` rules (N < 30 and N >= 30)

fmt <- function(name, val) {
  cat(sprintf("%-34s = %.17e\n", name, val))
}

cat("=== Regularized Incomplete Beta & Gamma ===\n")
fmt("ibeta(2.5, 3.5, 0.4)",      pbeta(0.4, 2.5, 3.5))
fmt("ibeta(0.5, 0.5, 0.25)",     pbeta(0.25, 0.5, 0.5))
fmt("ibeta(12.0, 8.0, 0.72)",    pbeta(0.72, 12.0, 8.0))
fmt("igamma(2.5, 1.8)",          pgamma(1.8, shape = 2.5, rate = 1.0))
fmt("igammac(2.5, 1.8)",         pgamma(1.8, shape = 2.5, rate = 1.0, lower.tail = FALSE))
fmt("igamma(10.0, 15.0)",        pgamma(15.0, shape = 10.0, rate = 1.0))
fmt("igammac(10.0, 15.0)",       pgamma(15.0, shape = 10.0, rate = 1.0, lower.tail = FALSE))

cat("\n=== Normal Distribution ===\n")
fmt("zdist(-1.96)",              pnorm(-1.96))
fmt("zdist(2.5)",                pnorm(2.5))
fmt("zinv(0.975)",               qnorm(0.975))
fmt("zinv(1e-6)",                qnorm(1e-6))
fmt("zpdf(1.5)",                 dnorm(1.5))
fmt("normdist(115, 100, 15)",    pnorm(115, mean = 100, sd = 15))
fmt("norminv(0.9, 100, 15)",     qnorm(0.9, mean = 100, sd = 15))
fmt("normpdf(115, 100, 15)",     dnorm(115, mean = 100, sd = 15))

cat("\n=== Student t Distribution ===\n")
fmt("tdist(2.22813885, 10)",     pt(2.22813885, df = 10, lower.tail = FALSE))
fmt("tdist(-1.5, 7.5)",          pt(-1.5, df = 7.5, lower.tail = FALSE))
fmt("tinv(0.05, 10)",            qt(1 - 0.05 / 2, df = 10))
fmt("tinv(0.01, 25)",            qt(1 - 0.01 / 2, df = 25))
fmt("tpdf(1.25, 8)",             dt(1.25, df = 8))

cat("\n=== Snedecor F Distribution ===\n")
fmt("fdist(3.5, 4, 20)",         pf(3.5, df1 = 4, df2 = 20, lower.tail = FALSE))
fmt("finv(0.05, 4, 20)",         qf(0.05, df1 = 4, df2 = 20, lower.tail = FALSE))
fmt("finv(0.01, 2, 15)",         qf(0.01, df1 = 2, df2 = 15, lower.tail = FALSE))
fmt("fpdf(2.4, 5, 12)",          df(2.4, df1 = 5, df2 = 12))

cat("\n=== Chi-Square Distribution ===\n")
fmt("chidist(9.487729, 4)",      pchisq(9.487729, df = 4, lower.tail = FALSE))
fmt("chisqc(9.487729, 4)",       pchisq(9.487729, df = 4, lower.tail = TRUE))
fmt("chiinv(0.05, 4)",           qchisq(0.05, df = 4, lower.tail = FALSE))
fmt("chiinv(0.01, 12)",          qchisq(0.01, df = 12, lower.tail = FALSE))

cat("\n=== Binomial Upper Cumulative Tail ===\n")
fmt("binomial(3, 10, 0.25)",     pbinom(2, size = 10, prob = 0.25, lower.tail = FALSE))
fmt("binomial(2.3, 10, 0.25)",   pbinom(2, size = 10, prob = 0.25, lower.tail = FALSE))
fmt("binomial(15, 25, 0.4)",     pbinom(14, size = 25, prob = 0.4, lower.tail = FALSE))
fmt("binomial(0, 10, 0.3)",      1.0)
fmt("binomial(11, 10, 0.3)",     0.0)

cat("\n=== Algebraic Helpers ===\n")
fmt("fact(0)",                   factorial(0))
fmt("fact(12)",                  factorial(12))
fmt("fact(100)",                 factorial(100))
fmt("logit(0.2)",                qlogis(0.2))
fmt("logit(0.999)",              qlogis(0.999))
fmt("probit(0.5)",               5.0 + qnorm(0.5))
fmt("probit(0.975)",             5.0 + qnorm(0.975))
fmt("probit(0.1)",               5.0 + qnorm(0.1))

cat("\n=== Modified Bessel I_n and K_n ===\n")
fmt("besseli(0, 2.5)",           besselI(2.5, 0))
fmt("besseli(1, 2.5)",           besselI(2.5, 1))
fmt("besseli(3, 2.5)",           besselI(2.5, 3))
fmt("besseli(-3, 2.5)",          besselI(2.5, 3))
fmt("besseli(3, -2.5)",         -besselI(2.5, 3))
fmt("besseli(4, -2.5)",          besselI(2.5, 4))
fmt("besseli(5, 30.0)",          besselI(30.0, 5))
fmt("besseli(10, 40.0)",         besselI(40.0, 10))
fmt("besselk(0, 1.5)",           besselK(1.5, 0))
fmt("besselk(1, 1.5)",           besselK(1.5, 1))
fmt("besselk(3, 1.5)",           besselK(1.5, 3))
fmt("besselk(-3, 1.5)",          besselK(1.5, 3))
fmt("besselk(5, 4.0)",           besselK(4.0, 5))

cat("\n=== Empirical OLS Slopes for [InitialValues] ===\n")
# N = 40 (k = floor(0.10 * 40) = 4), y = 0.05 * x^2 - 1.2 * x + 3
x40 <- seq(0, 39, by = 1)
y40 <- 0.05 * x40^2 - 1.2 * x40 + 3.0
fmt("slope_xmin_n40",            unname(coef(lm(y40[1:4] ~ x40[1:4]))[2]))
fmt("slope_xmid_n40",            unname(coef(lm(y40[19:22] ~ x40[19:22]))[2]))
fmt("slope_xmax_n40",            unname(coef(lm(y40[37:40] ~ x40[37:40]))[2]))

# N = 7 (odd, N < 30), y = x^2
x7 <- c(0, 1, 2, 3, 5, 7, 10)
y7 <- x7^2
fmt("slope_xmin_n7",             (y7[2] - y7[1]) / (x7[2] - x7[1]))
fmt("slope_xmid_n7",             unname(coef(lm(y7[3:5] ~ x7[3:5]))[2]))
fmt("slope_xmax_n7",             (y7[7] - y7[6]) / (x7[7] - x7[6]))

# N = 6 (even, N < 30), y = x^2
x6 <- c(0, 1, 2, 4, 7, 10)
y6 <- x6^2
fmt("slope_xmin_n6",             (y6[2] - y6[1]) / (x6[2] - x6[1]))
fmt("slope_xmid_n6",             (y6[4] - y6[3]) / (x6[4] - x6[3]))
fmt("slope_xmax_n6",             (y6[6] - y6[5]) / (x6[6] - x6[5]))
