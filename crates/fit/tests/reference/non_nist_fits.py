"""Generate the two numerical fitting references with mpmath 1.3.0.

Run in an isolated Python environment. The Rust tests use stored values offline.
An optional argument selects at least 80 decimal digits for a reproducibility check.
"""

import sys

from mpmath import mp

mp.dps = int(sys.argv[1]) if len(sys.argv) > 1 else 80
if mp.dps < 80:
    raise ValueError("use at least 80 decimal digits for the stationarity solve")


def show(label, values):
    print(label)
    print(", ".join(format(float(value), ".17g") for value in values))


def summarize(label, params, observed, fitted, jacobian, weights):
    residuals = observed - fitted
    rss = (residuals.T * residuals)[0]
    objective = (residuals.T * weights * residuals)[0]
    information = jacobian.T * weights * jacobian
    variance = objective / (len(observed) - len(params))
    covariance = mp.inverse(information) * variance
    show(label + " estimates", params)
    show(label + " SEs", [mp.sqrt(covariance[i, i]) for i in range(len(params))])
    show(label + " fitted", fitted)
    show(label + " RSS and objective", [rss, objective])


def weighted_gaussian():
    xs = list(map(mp.mpf, "1 1.6 2.3 3 3.7 4.4 5.2 6.1 7 8 9 10.25".split()))
    ys = mp.matrix(list(map(mp.mpf, "2.3 2.1 3.4 4.9 7.9 10.2 9.8 7.7 4.1 2.7 2.6 2.2".split())))
    weights = mp.diag([1 / x**2 for x in xs])

    def predictions_and_jacobian(baseline, amplitude, center, sigma):
        z = [(x - center) / sigma for x in xs]
        kernel = [mp.exp(-(q**2) / 2) for q in z]
        fitted = mp.matrix([baseline + amplitude * k for k in kernel])
        jacobian = mp.matrix(
            [[1, k, amplitude * k * q / sigma, amplitude * k * q**2 / sigma] for k, q in zip(kernel, z)]
        )
        return fitted, jacobian

    def score(*params):
        fitted, jacobian = predictions_and_jacobian(*params)
        return jacobian.T * weights * (fitted - ys)

    # Solve the weighted least-squares stationarity equations instead of
    # running the production Levenberg–Marquardt iteration.
    params = mp.findroot(score, (2, 8, 5, mp.mpf("1.5")), tol=mp.mpf("1e-60"))
    fitted, jacobian = predictions_and_jacobian(*params)
    summarize("Gaussian", params, ys, fitted, jacobian, weights)


def fixed_knot_segmental():
    xs = list(map(mp.mpf, "-1 0 0.75 1.75 2.5 3.25 4 5.5 7".split()))
    ys = mp.matrix(list(map(mp.mpf, "0.4 1.6 2.0 3.4 4.1 3.7 3.5 2.6 2.0".split())))
    join = mp.mpf("2.5")
    # With a fixed join the two-slope model is linear in the free parameters.
    design = mp.matrix([[1, min(x, join), max(0, x - join)] for x in xs])
    params, _ = mp.qr_solve(design, ys)
    summarize("Segmental", params, ys, design * params, design, mp.eye(len(xs)))


if __name__ == "__main__":
    weighted_gaussian()
    fixed_knot_segmental()
