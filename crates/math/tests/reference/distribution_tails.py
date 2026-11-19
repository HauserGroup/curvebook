"""Regenerate d1_distribution_tails.rs reference values with mpmath 1.3.0.

Run with an isolated Python environment. The Rust tests use stored binary64
values. The optional argument sets decimal precision (at least 120 digits).
"""

import sys

from mpmath import mp

mp.dps = int(sys.argv[1]) if len(sys.argv) > 1 else 120
if mp.dps < 120:
    raise ValueError("use at least 120 decimal digits to compute the beta complements")


def beta(a, b, x):
    # DLMF 8.17.1–2. Swap shapes and endpoints to integrate the upper tail.
    return mp.betainc(a, b, 0, x, regularized=True), mp.betainc(b, a, 0, 1 - x, regularized=True)


def gamma(a, x):
    # DLMF 8.2.1–4. Evaluate both integrals directly.
    return mp.gammainc(a, 0, x, regularized=True), mp.gammainc(a, x, regularized=True)


def t_tail(t, df, delta, upper):
    # Condition (Z + delta) / sqrt(V / df) on V, with V distributed as chi-square.
    # Integrate over u = sqrt(V / df) and use erfc for the requested Gaussian tail.
    shape = df / 2
    scale = 2 * shape**shape / mp.gamma(shape)
    sign = 1 if upper else -1

    def integrand(u):
        if u == 0:
            return mp.mpf(0)
        tail = mp.erfc(sign * (t * u - delta) / mp.sqrt(2)) / 2
        return tail * u ** (df - 1) * mp.exp(-shape * u * u)

    # With df >= 1.5 the omitted chi-square mass past u=32 is < 1e-330.
    return scale * mp.quad(integrand, [0, mp.mpf("0.25"), 1, 3, 6, 10, 16, 32])


def poisson_mixture(noncentrality, term):
    # Sum fresh central probabilities from k=0. Once past the mode, bound the
    # omitted mass by the geometric series of successive weight ratios.
    mean = noncentrality / 2
    weight = mp.exp(-mean)
    k = 0
    terms = [[], []]
    while True:
        lower, upper = term(k)
        terms[0].append(weight * lower)
        terms[1].append(weight * upper)
        k += 1
        weight *= mean / k
        ratio = mean / (k + 1)
        if ratio < 1 and weight / (1 - ratio) < mp.mpf("1e-100"):
            return tuple(mp.fsum(tail) for tail in terms)


def f_tails(f, df1, df2, noncentrality):
    x = (df1 * f) / (df1 * f + df2)
    return poisson_mixture(noncentrality, lambda k: beta(df1 / 2 + k, df2 / 2, x))


def chi2_tails(x, df, noncentrality):
    return poisson_mixture(noncentrality, lambda k: gamma(df / 2 + k, x / 2))


def show(label, *values):
    print(label, *(format(float(value), ".17g") for value in values), flush=True)


def tails(label, values, include_logs=False):
    show(label, *values, *((mp.log(tail) for tail in values) if include_logs else ()))


def invert(probability, forward, guesses):
    target = mp.mpf(probability)
    return mp.findroot(lambda x: forward(x) / target - 1, tuple(map(mp.mpf, guesses)), tol=mp.mpf("1e-90"))


def interval(label, forward, lower_guesses, upper_guesses):
    # At the lower noncentrality the SF is 0.025; at the upper the CDF is 0.025.
    show(label, invert("0.025", lambda ncp: forward(ncp, True), lower_guesses),
         invert("0.025", lambda ncp: forward(ncp, False), upper_guesses))


def main():
    tiny_beta = mp.mpf(2) ** -280
    for label, a, b, x in [
        ("beta small shape", mp.mpf("0.25"), 72, tiny_beta),
        ("beta underflow", 20, mp.mpf("1.25"), mp.mpf(2) ** -60),
        ("beta before fraction switch", mp.mpf("2.5"), mp.mpf("7.5"), mp.mpf(149) / 512),
        ("beta after fraction switch", mp.mpf("2.5"), mp.mpf("7.5"), mp.mpf(150) / 512),
    ]:
        tails(label, beta(a, b, x), include_logs=True)
    show("beta lower quantile", invert("0.1", lambda x: beta(mp.mpf("2.5"), mp.mpf("7.5"), x)[0], ("0.05", "0.1")))
    show("beta upper quantile", invert("0.1", lambda x: beta(mp.mpf("2.5"), mp.mpf("7.5"), x)[1], ("0.35", "0.45")))
    show("beta tiny quantile complement", mp.exp(invert("1e-20", lambda log_x: beta(mp.mpf("0.25"), 72, mp.exp(log_x))[0], ("-190", "-188"))))
    for label, a, x in [
        ("gamma small shape", mp.mpf("0.25"), mp.mpf(2) ** -280),
        ("gamma underflow", mp.mpf("0.375"), 850),
        ("gamma before Temme", 4095, 4111),
        ("gamma at Temme", 4096, 4112),
    ]:
        tails(label, gamma(a, x), include_logs=True)
    show("gamma lower quantile", mp.exp(invert("1e-20", lambda log_x: gamma(mp.mpf("0.25"), mp.exp(log_x))[0], ("-186", "-184"))))
    show("gamma upper quantile", invert("1e-60", lambda x: gamma(mp.mpf("0.375"), x)[1], ("133", "135")))

    for label, t, df, delta in [
        ("t small lower tail", mp.mpf("0.5"), mp.mpf("1.5"), 12),
        ("t small upper tail", mp.mpf("-0.75"), mp.mpf("7.5"), -11),
        ("t ordinary", mp.mpf("2.75"), mp.mpf(6), mp.mpf("1.25")),
    ]:
        tails(label, tuple(t_tail(t, df, delta, upper) for upper in (False, True)))
    for label, f, df1, df2, ncp in [
        ("F tiny lower tail", mp.mpf("0.125"), mp.mpf("1.5"), mp.mpf(40), mp.mpf(100)),
        ("F large noncentrality", mp.mpf(80), mp.mpf(5), mp.mpf(120), mp.mpf(320)),
    ]:
        tails(label, f_tails(f, df1, df2, ncp))
    for label, x, df, ncp in [
        ("chi-square small df", mp.mpf(3), mp.mpf("0.75"), mp.mpf(120)),
        ("chi-square tiny upper tail", mp.mpf(720), mp.mpf(8), mp.mpf(320)),
        ("chi-square ordinary", mp.mpf(48), mp.mpf(8), mp.mpf(40)),
    ]:
        tails(label, chi2_tails(x, df, ncp))

    t, df = mp.mpf("2.75"), mp.mpf(6)
    interval("t 95% interval", lambda ncp, upper: t_tail(t, df, ncp, upper),
             ("0.1", "0.5"), ("5", "6"))
    f, df1, df2 = map(mp.mpf, (80, 5, 120))
    interval("F 95% interval", lambda ncp, upper: f_tails(f, df1, df2, ncp)[upper], ("285", "300"), ("510", "525"))
    x, df = map(mp.mpf, (48, 8))
    interval("chi-square 95% interval", lambda ncp, upper: chi2_tails(x, df, ncp)[upper], ("17", "23"), ("67", "75"))


if __name__ == "__main__":
    main()
