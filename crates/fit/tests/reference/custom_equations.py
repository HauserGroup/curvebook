"""Generate independent D1 numerical references for custom equations with mpmath 1.3.0.

Run in an isolated Python environment. The Rust tests use stored values offline.
An optional CLI argument sets decimal precision (at least 80 digits) for
reproducibility checks between 80 and 120 digits.
"""

import sys

from mpmath import mp

mp.dps = int(sys.argv[1]) if len(sys.argv) > 1 else 80
if mp.dps < 80:
    raise ValueError("use at least 80 decimal digits for independent reference generation")


def fmt_vals(values):
    return ", ".join(format(float(v), ".17g") for v in values)


def bracketed_brent_solve(func, lo, hi, tol):
    """High-precision safeguarded Brent-Dekker root-finder on a sign-changing bracket."""
    xa = mp.mpf(lo)
    xb = mp.mpf(hi)
    fa = func(xa)
    fb = func(xb)
    if fa == 0:
        return xa
    if fb == 0:
        return xb
    if (fa > 0) == (fb > 0):
        raise ValueError("bracket endpoints do not have opposite signs")

    xc = xa
    fc = fa
    d = xb - xa
    e = d
    eps = mp.mpf(2) ** (-mp.prec)

    for _ in range(512):
        if (fb > 0) == (fc > 0):
            xc = xa
            fc = fa
            d = xb - xa
            e = d
        if abs(fc) < abs(fb):
            xa, xb, xc = xb, xc, xb
            fa, fb, fc = fb, fc, fb

        tol1 = 2 * eps * abs(xb) + mp.mpf("0.5") * tol
        xm = mp.mpf("0.5") * (xc - xb)
        if abs(xm) <= tol1 or fb == 0:
            return xb

        if abs(e) >= tol1 and abs(fa) > abs(fb):
            s = fb / fa
            if xa == xc:
                p = 2 * xm * s
                q = 1 - s
            else:
                q0 = fa / fc
                r = fb / fc
                p = s * (2 * xm * q0 * (q0 - r) - (xb - xa) * (r - 1))
                q = (q0 - 1) * (r - 1) * (s - 1)
            if p > 0:
                q = -q
            p = abs(p)
            min1 = 3 * xm * q - abs(tol1 * q)
            min2 = abs(e * q)
            if 2 * p < (min1 if min1 < min2 else min2):
                e = d
                d = p / q
            else:
                d = xm
                e = d
        else:
            d = xm
            e = d

        xa = xb
        fa = fb
        if abs(d) > tol1:
            xb = xb + d
        else:
            xb = xb + (tol1 if xm > 0 else -tol1)
        fb = func(xb)

    raise RuntimeError("bracketed Brent solver did not converge")


def implicit_cubic_roots():
    a = mp.mpf(2.5)
    # Use the exact IEEE-754 binary64 values corresponding to the Rust literals.
    xs_raw = [-3.0, -0.4, -1e-9, 0.0, 1e-9, 0.4, 3.0]
    scales_raw = [
        1e-200,
        1e-100,
        1e-50,
        1e-24,
        1e-12,
        1e-6,
        0.000125,
        1.0,
        7.5,
        1e6,
        1e12,
        1e24,
        1e50,
        1e100,
        1e200,
    ]
    tol = mp.power(10, -(mp.dps - 12))
    roots = []
    cardano_diffs = []
    residuals = []
    max_scale_diff = mp.mpf(0)

    for x_f64 in xs_raw:
        x = mp.mpf(x_f64)
        rhs = a * x
        if rhs == 0:
            root = mp.mpf(0)
        else:
            bound = 1 + abs(rhs) + mp.cbrt(abs(rhs))
            root = bracketed_brent_solve(lambda y: y + y**3 - rhs, -bound, bound, tol)
            for s_f64 in scales_raw:
                s = mp.mpf(s_f64)
                scaled_root = bracketed_brent_solve(
                    lambda y, sc=s: sc * (y + y**3 - rhs), -bound, bound, tol
                )
                max_scale_diff = max(max_scale_diff, abs(scaled_root - root))

        # Cross-check against Cardano's real cubic root formula for Y^3 + Y - rhs = 0.
        disc = mp.sqrt((rhs / 2) ** 2 + mp.mpf(1) / 27)
        u_plus = rhs / 2 + disc
        u_minus = rhs / 2 - disc
        real_cbrt = lambda v: mp.sign(v) * mp.cbrt(abs(v)) if v != 0 else mp.mpf(0)
        cardano = real_cbrt(u_plus) + real_cbrt(u_minus)
        cardano_diffs.append(abs(root - cardano))
        residuals.append(abs(root + root**3 - rhs))
        roots.append(root)

    print("=== Implicit Cubic Y + Y^3 = A * X (A = 2.5) ===")
    print("predictors:", fmt_vals(xs_raw))
    print("roots:     ", fmt_vals(roots))
    print("max |root - cardano|:", format(float(max(cardano_diffs)), ".3e"))
    print("max |residual| at dps:", format(float(max(residuals)), ".3e"))
    print("max |scaled - unscaled| across [1e-200, 1e200]:", format(float(max_scale_diff), ".3e"))


def forced_ode_closed_form():
    t0 = mp.mpf("1.5")
    times = [-0.5, 0.25, 1.0, 1.5, 2.25, 3.5, 5.0]
    param_sets = [
        ("P0", [0.6, 2.5, 1.8, 1.1, 3.0]),
        ("P1", [1.25, -1.5, 2.5, 0.4, -2.0]),
        ("P2", [0.35, 1.2, 0.9, 0.85, 4.5]),
    ]

    def eval_ds0(t, p):
        k, q, w, _r, y0 = [mp.mpf(v) for v in p]
        dt = t - t0
        denom = k * k + w * w
        p_t = (k * mp.cos(w * t) + w * mp.sin(w * t)) / denom
        p_0 = (k * mp.cos(w * t0) + w * mp.sin(w * t0)) / denom
        return (y0 - q * p_0) * mp.exp(-k * dt) + q * p_t

    def eval_ds1(t, p):
        k, q, _w, r, y0 = [mp.mpf(v) for v in p]
        dt = t - t0
        return y0 * mp.exp(-k * dt) + (q / (k - r)) * (mp.exp(-r * dt) - mp.exp(-k * dt))

    print("\n=== Forced ODE with t0 = 1.5 ===")
    print("times:", fmt_vals(times))
    for label, p in param_sets:
        vals_a = [eval_ds0(mp.mpf(t), p) for t in times]
        vals_b = [eval_ds1(mp.mpf(t), p) for t in times]
        print(f"{label} DS0 (<A> cosine forcing):", fmt_vals(vals_a))
        print(f"{label} DS1 (<B> exp forcing):   ", fmt_vals(vals_b))


def two_dataset_shared_rate_ode_fit():
    xs_str = ["0.0", "0.5", "1.0", "2.0", "3.5", "5.0", "7.5", "10.0"]
    y_a_str = ["79.1", "68.4", "55.6", "40.7", "22.9", "14.6", "5.4", "2.8"]
    y_b_str = ["35.8", "28.2", "25.3", "16.9", "10.8", "5.7", "2.8", "0.9"]
    xs = [mp.mpf(v) for v in xs_str]
    y_a = mp.matrix([mp.mpf(v) for v in y_a_str])
    y_b = mp.matrix([mp.mpf(v) for v in y_b_str])
    y_all = mp.matrix(list(y_a) + list(y_b))

    def fitted_and_jacobian(k, a0, a1):
        exp_x = [mp.exp(-k * x) for x in xs]
        rows_f = [a0 * e for e in exp_x] + [a1 * e for e in exp_x]
        jac = mp.zeros(2 * len(xs), 3)
        for i, (x, e) in enumerate(zip(xs, exp_x)):
            jac[i, 0] = -a0 * x * e
            jac[i, 1] = e
            jac[i + len(xs), 0] = -a1 * x * e
            jac[i + len(xs), 2] = e
        return mp.matrix(rows_f), jac

    def normal_eqs(k, a0, a1):
        fitted, jac = fitted_and_jacobian(k, a0, a1)
        return jac.T * (fitted - y_all)

    params = mp.findroot(normal_eqs, (mp.mpf("0.35"), mp.mpf("80.0"), mp.mpf("35.0")), tol=mp.mpf("1e-60"))
    k_hat, a0_hat, a1_hat = params
    fitted, jac = fitted_and_jacobian(k_hat, a0_hat, a1_hat)
    res = y_all - fitted
    res_a = res[: len(xs)]
    res_b = res[len(xs) :]
    rss_a = sum(r * r for r in res_a)
    rss_b = sum(r * r for r in res_b)
    rss_total = rss_a + rss_b
    df = 2 * len(xs) - 3
    cov = mp.inverse(jac.T * jac) * (rss_total / df)
    ses = [mp.sqrt(cov[i, i]) for i in range(3)]

    print("\n=== Two-dataset shared-rate ODE fit (Y0 = A, dY/dX = -k * Y) ===")
    print("X:  ", ", ".join(xs_str))
    print("DS1:", ", ".join(y_a_str))
    print("DS2:", ", ".join(y_b_str))
    print("estimates [k, A_DS1, A_DS2]:", fmt_vals([k_hat, a0_hat, a1_hat]))
    print("SEs       [k, A_DS1, A_DS2]:", fmt_vals(ses))
    print("fitted DS1:", fmt_vals(fitted[: len(xs)]))
    print("fitted DS2:", fmt_vals(fitted[len(xs) :]))
    print("RSS [total, DS1, DS2]:", fmt_vals([rss_total, rss_a, rss_b]))


if __name__ == "__main__":
    implicit_cubic_roots()
    forced_ode_closed_form()
    two_dataset_shared_rate_ode_fit()
