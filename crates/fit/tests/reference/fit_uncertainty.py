"""Generate independent high-precision fitting uncertainty oracles with mpmath 1.3.0.

Run in an isolated Python environment. The Rust integration tests in
`crates/fit/tests/d1_fit_uncertainty.rs` store the verified binary64 values offline.
An optional CLI argument sets decimal working precision (`dps >= 80`, default 80)
for reproducibility checks (e.g. comparing 80 vs 120 digits).
"""

import sys

from mpmath import mp

dps = int(sys.argv[1]) if len(sys.argv) > 1 else 80
if dps < 80:
    raise ValueError("use at least 80 decimal digits for independent reference solves")
mp.dps = dps


def show(label, values):
    print(label)
    print(", ".join(format(float(v), ".17g") for v in values))


def show_matrix(label, mat):
    print(label)
    for row in range(mat.rows):
        print("  [" + ", ".join(format(float(mat[row, col]), ".17g") for col in range(mat.cols)) + "]")


def chi2_1_isf(alpha):
    """Exact 1-df chi-square upper quantile chi2_{1, 1-alpha} via inverse error function."""
    return 2 * mp.erfinv(1 - alpha) ** 2


def student_t_two_sided_isf(alpha, df):
    """Exact two-sided Student-t critical value t_{df, 1-alpha/2} via regularized incomplete beta."""
    nu = mp.mpf(df)
    target = mp.mpf(alpha)

    def tail_eq(t):
        return mp.betainc(nu / 2, mp.mpf("0.5"), 0, nu / (nu + t * t), regularized=True) - target

    return mp.findroot(tail_eq, mp.mpf("2.2"), tol=mp.mpf("1e-60"))


def poisson_log_linear_fixture():
    """Poisson log-linear mean mu(x) = exp(Intercept + Slope * x) with zero counts."""
    xs = [mp.mpf(x) for x in [0, 1, 2, 3, 4, 5, 6, 7, 8, 9]]
    ys = [mp.mpf(y) for y in [0, 1, 0, 2, 1, 4, 3, 7, 9, 14]]
    n_obs = len(xs)
    k_free = 2
    df = n_obs - k_free
    sum_y = sum(ys)
    sum_xy = sum(x * y for x, y in zip(xs, ys))
    ln_sum_xy = mp.log(sum_xy)

    def intercept_given_slope(slope):
        # Closed-form nuisance MLE from d(ell)/d(Intercept) = sum(y) - exp(Intercept) * sum(exp(Slope * x)) = 0.
        return mp.log(sum_y / sum(mp.exp(slope * x) for x in xs))

    def slope_given_intercept(intercept):
        # Monotone score d(ell)/d(Slope) = sum(x * y) - exp(Intercept) * sum(x * exp(Slope * x)) = 0,
        # solved in log-sum space for global numerical stability across wide intercept values.
        target = ln_sum_xy - intercept
        return mp.findroot(
            lambda s: mp.log(sum(x * mp.exp(s * x) for x in xs if x > 0)) - target,
            mp.mpf("0.4"),
            tol=mp.mpf("1e-60"),
        )

    def deviance(intercept, slope):
        total = mp.mpf(0)
        for x, y in zip(xs, ys):
            mu = mp.exp(intercept + slope * x)
            if y == 0:
                total += 2 * mu
            else:
                total += 2 * (y * mp.log(y / mu) - (y - mu))
        return total

    # Full-MLE score wrt Slope after substituting closed-form Intercept(Slope).
    def mle_score(slope):
        intercept = intercept_given_slope(slope)
        return sum_xy - sum(x * mp.exp(intercept + slope * x) for x in xs)

    slope_hat = mp.findroot(mle_score, mp.mpf("0.4"), tol=mp.mpf("1e-60"))
    intercept_hat = intercept_given_slope(slope_hat)
    mu_hat = [mp.exp(intercept_hat + slope_hat * x) for x in xs]
    dev_min = deviance(intercept_hat, slope_hat)
    rss = sum((y - mu) ** 2 for y, mu in zip(ys, mu_hat))

    # Poisson likelihood and information criteria (dispersion phi = 1, K = k_free = 2).
    ll = sum(
        (y * mp.log(mu) if y > 0 else mp.mpf(0)) - mu - mp.loggamma(y + 1)
        for y, mu in zip(ys, mu_hat)
    )
    neg2_ll = -2 * ll
    k = mp.mpf(k_free)
    n = mp.mpf(n_obs)
    aicc = neg2_ll + 2 * k + (2 * k * (k + 1)) / (n - k - 1)
    bic = neg2_ll + k * mp.log(n)

    # Fisher information X^T diag(mu_hat) X (fixed dispersion phi = 1).
    design = mp.matrix([[1, x] for x in xs])
    w_mu = mp.diag(mu_hat)
    cov_fisher = mp.inverse(design.T * w_mu * design)
    se_intercept = mp.sqrt(cov_fisher[0, 0])
    se_slope = mp.sqrt(cov_fisher[1, 1])

    print("=== Fixture 1: Poisson Log-Linear Fit & Profile-Deviance CIs ===")
    show("Poisson MLE [Intercept, Slope]", [intercept_hat, slope_hat])
    show("Poisson Wald SEs [Intercept, Slope]", [se_intercept, se_slope])
    show("Poisson fitted means mu_hat", mu_hat)
    show("Poisson [df, dev_min, rss, ll, aicc, bic]", [df, dev_min, rss, ll, aicc, bic])
    show_matrix("Poisson Fisher Covariance [[Intercept, Slope]]", cov_fisher)

    for conf_str, alpha_str in [("95", "0.05"), ("99", "0.01")]:
        alpha = mp.mpf(alpha_str)
        chi2_cut = chi2_1_isf(alpha)
        dev_crit = dev_min + chi2_cut

        # Profile Slope with closed-form nuisance Intercept(Slope).
        prof_slope = lambda s: deviance(intercept_given_slope(s), s) - dev_crit
        s_lo = mp.findroot(prof_slope, (mp.mpf("0.05"), slope_hat), solver="anderson", tol=mp.mpf("1e-50"))
        s_hi = mp.findroot(prof_slope, (slope_hat, mp.mpf("1.0")), solver="anderson", tol=mp.mpf("1e-50"))
        b0_at_s_lo = intercept_given_slope(s_lo)
        b0_at_s_hi = intercept_given_slope(s_hi)

        # Profile Intercept with reoptimized nuisance Slope(Intercept).
        prof_intercept = lambda b0: deviance(b0, slope_given_intercept(b0)) - dev_crit
        b0_lo = mp.findroot(prof_intercept, (mp.mpf("-4.0"), intercept_hat), solver="anderson", tol=mp.mpf("1e-50"))
        b0_hi = mp.findroot(prof_intercept, (intercept_hat, mp.mpf("1.0")), solver="anderson", tol=mp.mpf("1e-50"))
        s_at_b0_lo = slope_given_intercept(b0_lo)
        s_at_b0_hi = slope_given_intercept(b0_hi)

        # Sanity verification inside generator: profile increment equals chi2 cutoff to 45+ digits.
        for label, d_val in [
            (f"{conf_str}% slope lo", deviance(b0_at_s_lo, s_lo) - dev_min),
            (f"{conf_str}% slope hi", deviance(b0_at_s_hi, s_hi) - dev_min),
            (f"{conf_str}% intercept lo", deviance(b0_lo, s_at_b0_lo) - dev_min),
            (f"{conf_str}% intercept hi", deviance(b0_hi, s_at_b0_hi) - dev_min),
        ]:
            assert abs(d_val - chi2_cut) < mp.mpf("1e-45"), f"{label} deviance increment mismatch"

        print(f"-- Confidence {conf_str}% --")
        show(f"Poisson {conf_str}% chi2_1 cutoff & dev_crit", [chi2_cut, dev_crit])
        show(f"Poisson {conf_str}% Slope profile [lo, hi, b0_lo_nuis, b0_hi_nuis]", [s_lo, s_hi, b0_at_s_lo, b0_at_s_hi])
        show(f"Poisson {conf_str}% Intercept profile [lo, hi, s_lo_nuis, s_hi_nuis]", [b0_lo, b0_hi, s_at_b0_lo, s_at_b0_hi])


def shared_weighted_gaussian_fixture():
    """Globally shared slope + local intercepts with 1/|X| weights and unequal dataset sizes."""
    xs_a = [mp.mpf(x) for x in [1, 2, 4, 6, 8, 10, 13, 16]]
    ys_a = [mp.mpf(y) for y in ["5.4", "7.1", "11.5", "14.2", "19.1", "21.8", "28.9", "34.2"]]
    xs_b = [mp.mpf(x) for x in [-11, -8, -6, -4, -2, -1]]
    ys_b = [mp.mpf(y) for y in ["-25.4", "-18.1", "-15.2", "-11.0", "-6.3", "-4.9"]]

    design_rows = []
    y_vals = []
    weights = []
    for x, y in zip(xs_a, ys_a):
        design_rows.append([x, mp.mpf(1), mp.mpf(0)])
        y_vals.append(y)
        weights.append(1 / abs(x))
    for x, y in zip(xs_b, ys_b):
        design_rows.append([x, mp.mpf(0), mp.mpf(1)])
        y_vals.append(y)
        weights.append(1 / abs(x))

    n_obs = len(y_vals)
    k_free = 3
    df = n_obs - k_free
    x_mat = mp.matrix(design_rows)
    y_vec = mp.matrix(y_vals)
    w_mat = mp.diag(weights)

    # Normal equations for weighted linear design (Slope, YIntercept_A, YIntercept_B).
    xtwx = x_mat.T * w_mat * x_mat
    xtwy = x_mat.T * w_mat * y_vec
    theta = mp.lu_solve(xtwx, xtwy)
    fitted = x_mat * theta
    residuals = y_vec - fitted
    weighted_ss = (residuals.T * w_mat * residuals)[0]
    rss = (residuals.T * residuals)[0]
    sigma2 = weighted_ss / df
    xtwx_inv = mp.inverse(xtwx)
    cov = sigma2 * xtwx_inv
    ses = [mp.sqrt(cov[i, i]) for i in range(k_free)]

    t_crit = student_t_two_sided_isf("0.05", df)
    ci_lo_t = [theta[i] - t_crit * ses[i] for i in range(k_free)]
    ci_hi_t = [theta[i] + t_crit * ses[i] for i in range(k_free)]

    print("\n=== Fixture 2: Shared Weighted Gaussian (1/|X|) & Bands ===")
    show("Shared Gaussian [Slope, Intercept_A, Intercept_B]", [theta[0], theta[1], theta[2]])
    show("Shared Gaussian SEs [Slope, Intercept_A, Intercept_B]", ses)
    show("Shared Gaussian 95% t-CI lower [Slope, Intercept_A, Intercept_B]", ci_lo_t)
    show("Shared Gaussian 95% t-CI upper [Slope, Intercept_A, Intercept_B]", ci_hi_t)
    show("Shared Gaussian fitted DS0", list(fitted)[: len(xs_a)])
    show("Shared Gaussian fitted DS1", list(fitted)[len(xs_a) :])
    show("Shared Gaussian [df, weighted_ss, rss, sigma2, t_crit_95]", [df, weighted_ss, rss, sigma2, t_crit])
    show_matrix("Shared Gaussian Free Covariance (Slope, Intercept_A, Intercept_B)", cov)

    # Exact 256-point overlay grid indices [0, 51, 102, 153, 204, 255] correspond to fractions
    # 0/5, 1/5, 2/5, 3/5, 4/5, 5/5 -> integer X in both overlays:
    #   DS A [1, 16]: x = [1, 4, 7, 10, 13, 16]
    #   DS B [-11, -1]: x = [-11, -9, -7, -5, -3, -1]
    overlay_xs = {
        "DS0": ([mp.mpf(x) for x in [1, 4, 7, 10, 13, 16]], [1, 0]),
        "DS1": ([mp.mpf(x) for x in [-11, -9, -7, -5, -3, -1]], [0, 1]),
    }
    for ds_name, (grid_xs, ds_ind) in overlay_xs.items():
        print(f"-- Overlay bands {ds_name} --")
        for x in grid_xs:
            g = mp.matrix([x, mp.mpf(ds_ind[0]), mp.mpf(ds_ind[1])])
            y_pred = (g.T * theta)[0]
            var_mean = (g.T * cov * g)[0]
            se_mean = mp.sqrt(var_mean)
            # Future observation variance under 1/|X| weighting is sigma2 / w(x) = sigma2 * |x|.
            var_future = sigma2 * abs(x)
            se_pred = mp.sqrt(var_future + var_mean)
            ci_lo = y_pred - t_crit * se_mean
            ci_hi = y_pred + t_crit * se_mean
            pi_lo = y_pred - t_crit * se_pred
            pi_hi = y_pred + t_crit * se_pred
            show(
                f"  {ds_name} x={int(x)} [y_hat, ci_lo, ci_hi, pi_lo, pi_hi, se_mean, se_pred]",
                [y_pred, ci_lo, ci_hi, pi_lo, pi_hi, se_mean, se_pred],
            )


if __name__ == "__main__":
    print(f"mp.dps = {mp.dps}")
    poisson_log_linear_fixture()
    shared_weighted_gaussian_fixture()
