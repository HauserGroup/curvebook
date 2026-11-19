"""Pre-specified independent reference generator for d1_randomization.rs (R9.14).

Computes exact integer quotas (Hamilton / largest-remainder and round-robin
equal allocation), exact Binomial and Poisson CDFs at preselected integer
cutoffs, theoretical moments, statistical acceptance bounds, and exact
family-wise false-positive probabilities using mpmath 1.3.0.

Run with an isolated Python environment:
  python randomization.py 120
  python randomization.py 80
"""

from __future__ import annotations

import sys
from dataclasses import dataclass
from mpmath import mp


def hamilton_apportion(total: int, weights: list[int]) -> list[int]:
    """Exact integer Hamilton (largest-remainder) apportionment.

    Ties in remainder are broken by ascending arm index, matching CurveBook's
    documented tie-break convention.
    """
    k = len(weights)
    if total == 0 or k == 0:
        return [0] * k
    total_weight = sum(weights)
    alloc = [0] * k
    remainders: list[tuple[int, int]] = []
    assigned = 0
    for idx, w in enumerate(weights):
        num = total * w
        base = num // total_weight
        rem = num % total_weight
        alloc[idx] = base
        assigned += base
        remainders.append((idx, rem))
    leftover = total - assigned
    remainders.sort(key=lambda item: (-item[1], item[0]))
    for idx, _ in remainders[:leftover]:
        alloc[idx] += 1
    return alloc


def apportion_stratified_proportional(sample_size: int, stratum_sizes: list[int]) -> list[int]:
    """Exact iterative proportional stratum apportionment with min-1 and capacity clamps."""
    s_count = len(stratum_sizes)
    alloc = [0] * s_count
    if sample_size == 0 or s_count == 0:
        return alloc
    enforce_min_one = sample_size >= s_count
    active = list(range(s_count))
    remaining = sample_size
    while active and remaining > 0:
        active_weights = [stratum_sizes[i] for i in active]
        proposal = hamilton_apportion(remaining, active_weights)
        clamped_any = False
        next_active: list[int] = []
        for s_idx, proposed in zip(active, proposal):
            cap = stratum_sizes[s_idx]
            if enforce_min_one and proposed == 0 and cap >= 1:
                alloc[s_idx] = 1
                remaining -= 1
                clamped_any = True
            elif proposed > cap:
                alloc[s_idx] = cap
                remaining -= cap
                clamped_any = True
            else:
                next_active.append(s_idx)
        if not clamped_any:
            for s_idx, proposed in zip(active, proposal):
                alloc[s_idx] = proposed
            break
        active = next_active
    return alloc


def apportion_stratified_equal(sample_size: int, stratum_sizes: list[int]) -> list[int]:
    """Exact round-robin water-filling equal stratum apportionment."""
    s_count = len(stratum_sizes)
    alloc = [0] * s_count
    remaining = sample_size
    while remaining > 0:
        unsaturated = [i for i in range(s_count) if alloc[i] < stratum_sizes[i]]
        if not unsaturated:
            break
        step = remaining // len(unsaturated)
        if step == 0:
            for idx in unsaturated[:remaining]:
                alloc[idx] += 1
            break
        for idx in unsaturated:
            add = min(step, stratum_sizes[idx] - alloc[idx])
            alloc[idx] += add
            remaining -= add
    return alloc


def exact_binom_cdf_sum(n: int, p: mp.mpf, k: int) -> mp.mpf:
    """Exact Binomial(n, p) CDF P(X <= k) by summing exact PMF terms."""
    if k < 0:
        return mp.mpf(0)
    if k >= n:
        return mp.mpf(1)
    q = 1 - p
    terms = []
    # Use symmetry if k > n / 2 to minimize term count and cancellation.
    if k <= n // 2:
        # Start from j = 0: C(n,0) p^0 q^n
        term = q**n
        terms.append(term)
        for j in range(1, k + 1):
            term = term * (n - j + 1) / j * (p / q)
            terms.append(term)
        val = mp.fsum(terms)
    else:
        # P(X <= k) = 1 - sum_{j=k+1}^n C(n, j) p^j q^(n-j)
        term = p**n
        terms.append(term)
        for j in range(n - 1, k, -1):
            term = term * (j + 1) / (n - j) * (q / p)
            terms.append(term)
        val = 1 - mp.fsum(terms)
    # Cross-check against incomplete beta identity I_{1-p}(n-k, k+1)
    beta_val = mp.betainc(n - k, k + 1, 0, 1 - p, regularized=True)
    assert abs(val - beta_val) < mp.mpf("1e-50"), f"binom mismatch: {val} vs {beta_val}"
    return val


def exact_poisson_cdf_sum(lam: mp.mpf, k: int) -> mp.mpf:
    """Exact Poisson(lam) CDF P(X <= k) by summing exact PMF terms."""
    if k < 0:
        return mp.mpf(0)
    if lam == 0:
        return mp.mpf(1)
    term = mp.exp(-lam)
    terms = [term]
    for j in range(1, k + 1):
        term = term * lam / j
        terms.append(term)
    val = mp.fsum(terms)
    gamma_val = mp.gammainc(k + 1, lam, regularized=True)
    assert abs(val - gamma_val) < mp.mpf("1e-50"), f"poisson mismatch: {val} vs {gamma_val}"
    return val


def _log_binom_pmf(n: int, k: int, p: mp.mpf) -> mp.mpf:
    """Exact log P(C = k) for C ~ Binomial(n, p)."""
    if k == 0:
        return n * mp.log1p(-p)
    if k == n:
        return n * mp.log(p)
    return mp.loggamma(n + 1) - mp.loggamma(k + 1) - mp.loggamma(n - k + 1) + k * mp.log(p) + (n - k) * mp.log1p(-p)


def exact_binom_two_sided_tail(n: int, p: mp.mpf, lo_inclusive: int, hi_inclusive: int) -> mp.mpf:
    """Exact P(C < lo_inclusive) + P(C > hi_inclusive) for C ~ Binomial(n, p) by tail term summation."""
    q = 1 - p
    left = mp.mpf(0)
    if lo_inclusive > 0:
        c = lo_inclusive - 1
        term = mp.exp(_log_binom_pmf(n, c, p))
        terms = [term]
        for j in range(c, 0, -1):
            ratio = (j * q) / ((n - j + 1) * p)
            term *= ratio
            terms.append(term)
            if ratio < 1 and term / (1 - ratio) < mp.mpf("1e-40"):
                break
        left = mp.fsum(terms)

    right = mp.mpf(0)
    if hi_inclusive < n:
        c = hi_inclusive + 1
        term = mp.exp(_log_binom_pmf(n, c, p))
        terms = [term]
        for j in range(c, n):
            ratio = ((n - j) * p) / ((j + 1) * q)
            term *= ratio
            terms.append(term)
            if ratio < 1 and term / (1 - ratio) < mp.mpf("1e-40"):
                break
        right = mp.fsum(terms)
    return left + right


def exact_poisson_two_sided_tail(lam: mp.mpf, lo_inclusive: int, hi_inclusive: int) -> mp.mpf:
    """Exact P(S < lo_inclusive) + P(S > hi_inclusive) for S ~ Poisson(lam) by tail term summation."""
    left = mp.mpf(0)
    if lo_inclusive > 0:
        c = lo_inclusive - 1
        term = mp.exp(c * mp.log(lam) - lam - mp.loggamma(c + 1))
        terms = [term]
        for j in range(c, 0, -1):
            ratio = j / lam
            term *= ratio
            terms.append(term)
            if ratio < 1 and term / (1 - ratio) < mp.mpf("1e-40"):
                break
        left = mp.fsum(terms)

    right = mp.mpf(0)
    c = hi_inclusive + 1
    term = mp.exp(c * mp.log(lam) - lam - mp.loggamma(c + 1))
    terms = [term]
    j = c
    while True:
        ratio = lam / (j + 1)
        term *= ratio
        terms.append(term)
        j += 1
        if ratio < 1 and term / (1 - ratio) < mp.mpf("1e-40"):
            break
    right = mp.fsum(terms)
    return left + right


def chernoff_binom_two_sided_bound(n: int, p: mp.mpf, lo_inclusive: int, hi_inclusive: int) -> mp.mpf:
    """Non-asymptotic Chernoff KL upper bound on P(C < lo_inclusive) + P(C > hi_inclusive)."""
    bound = mp.mpf(0)
    if lo_inclusive > 0:
        a = mp.mpf(lo_inclusive - 1) / n
        if a < p:
            kl = (a * mp.log(a / p) if a > 0 else mp.mpf(0)) + (1 - a) * mp.log((1 - a) / (1 - p))
            bound += mp.exp(-n * kl)
    if hi_inclusive < n:
        b = mp.mpf(hi_inclusive + 1) / n
        if b > p:
            kl = b * mp.log(b / p) + ((1 - b) * mp.log((1 - b) / (1 - p)) if b < 1 else mp.mpf(0))
            bound += mp.exp(-n * kl)
    return bound


def chernoff_poisson_two_sided_bound(lam: mp.mpf, lo_inclusive: int, hi_inclusive: int) -> mp.mpf:
    """Non-asymptotic Chernoff upper bound on P(S < lo_inclusive) + P(S > hi_inclusive)."""
    bound = mp.mpf(0)
    if lo_inclusive > 0:
        a = mp.mpf(lo_inclusive - 1)
        if a < lam:
            kl = (a * mp.log(a / lam) if a > 0 else mp.mpf(0)) + lam - a
            bound += mp.exp(-kl)
    b = mp.mpf(hi_inclusive + 1)
    if b > lam:
        kl = b * mp.log(b / lam) + lam - b
        bound += mp.exp(-kl)
    return bound


@dataclass(frozen=True)
class DistCase:
    name: str
    family: str  # "binomial" or "poisson"
    param1: int | str  # trials or lambda str
    param2: str | None  # p str or None
    cutoffs: tuple[int, ...]
    regime: str


# Pre-specified stochastic cases, cutoffs, draw count, seed, and z-multipliers.
N_DRAWS = 25_000
STOCHASTIC_SEED = 2026_1118
ENDPOINT_N_DRAWS = 512
ENDPOINT_SEED = 2026_0001
Z_CDF = mp.mpf("5.6")
Z_MEAN = mp.mpf("5.6")
Z_VAR = mp.mpf("5.6")

CASES: tuple[DistCase, ...] = (
    DistCase("Binom32_Bernoulli", "binomial", 32, "0.375", (8, 10, 12, 15), "trials <= 32 (Bernoulli loop)"),
    DistCase("Binom33_Btrs", "binomial", 33, "0.375", (8, 11, 12, 16), "trials > 32, n*q = 12.375 >= 10 (BTRS)"),
    DistCase("Binom39_Geo", "binomial", 39, "0.25", (6, 8, 10, 13), "trials > 32, n*q = 9.75 < 10 (Geo)"),
    DistCase("Binom40_Btrs", "binomial", 40, "0.25", (6, 9, 10, 14), "trials > 32, n*q = 10.0 >= 10 (BTRS)"),
    DistCase("Binom2000_Geo", "binomial", 2000, "0.004", (4, 7, 8, 12), "large-n rare event n*q = 8.0 < 10 (Geo)"),
    DistCase("Binom60_FlipBtrs", "binomial", 60, "0.75", (40, 44, 45, 49), "p > 0.5 complement flip, n*q = 15 >= 10 (BTRS)"),
    DistCase("Binom64_FlipGeo", "binomial", 64, "0.875", (52, 55, 56, 59), "p > 0.5 complement flip, n*q = 8 < 10 (Geo)"),
    DistCase("Pois4p5_Small", "poisson", "4.5", None, (1, 3, 4, 7), "lambda < 30 (product of uniforms)"),
    DistCase("Pois29p5_Below30", "poisson", "29.5", None, (22, 27, 29, 37), "lambda < 30 boundary (product of uniforms)"),
    DistCase("Pois30_At30", "poisson", "30.0", None, (22, 27, 30, 37), "lambda >= 30 boundary (Hormann PTRS)"),
    DistCase("Pois120_Large", "poisson", "120.0", None, (105, 115, 120, 134), "lambda >= 30 large rate (PTRS + Stirling)"),
)


def fmt64(val: mp.mpf) -> str:
    return format(float(val), ".17g")


def main() -> None:
    dps = int(sys.argv[1]) if len(sys.argv) > 1 else 120
    if dps < 80:
        raise ValueError("use at least 80 decimal digits")
    mp.dps = dps

    print(f"# Reference generation with mpmath {mp.__version__ if hasattr(mp, '__version__') else '1.3.0'} at mp.dps={mp.dps}")
    print("\n## 1. Subject Randomization Exact Quotas")
    cb_4 = hamilton_apportion(15, [4, 3, 2, 1])
    cb_3 = hamilton_apportion(9, [3, 2, 2])
    print(f"cb_tied_4arm (N=15, weights=[4,3,2,1], seed=104729): {cb_4}")
    print(f"cb_tied_3arm (N=9, weights=[3,2,2], seed=224737): {cb_3}")
    print("pb_fixed (N=29, weights=[1,2,3], block_sizes=[12], seed=350377): full_block=[2, 4, 6], last_block_len=5")
    print("pb_variable (N=41, weights=[1,2,3], block_sizes=[6, 12], seed=479001): full_block_mult_counts=[1*m, 2*m, 3*m]")

    print("\n## 2. Stratified Row Sampling Exact Quotas (strata sizes [20, 12, 5, 2], 39 active + 4 ineligible)")
    sizes = [20, 12, 5, 2]
    print(f"strat_prop_15 (sample=15, seed=611953): {apportion_stratified_proportional(15, sizes)}")
    print(f"strat_prop_13 (sample=13, seed=746773): {apportion_stratified_proportional(13, sizes)}")
    print(f"strat_equal_19 (sample=19, seed=882377): {apportion_stratified_equal(19, sizes)}")

    print(f"\n## 3. Distribution CDFs, Moments, and FWER Budget (N={N_DRAWS}, seed={STOCHASTIC_SEED})")
    total_cdf_fp = mp.mpf(0)
    total_mean_fp = mp.mpf(0)
    total_var_budget = mp.mpf(0)
    n_cdf_checks = 0
    n_mean_checks = 0
    n_var_checks = 0

    for case in CASES:
        if case.family == "binomial":
            n = int(case.param1)
            p = mp.mpf(case.param2)
            mean = n * p
            var = n * p * (1 - p)
            ex_kurt = (1 - 6 * p * (1 - p)) / var
            cdfs = [exact_binom_cdf_sum(n, p, k) for k in case.cutoffs]
        else:
            lam = mp.mpf(case.param1)
            mean = lam
            var = lam
            ex_kurt = 1 / lam
            cdfs = [exact_poisson_cdf_sum(lam, k) for k in case.cutoffs]

        mean_se = mp.sqrt(var / N_DRAWS)
        mean_tol = Z_MEAN * mean_se
        var_se = var * mp.sqrt(mp.mpf(2) / (N_DRAWS - 1) + ex_kurt / N_DRAWS)
        var_tol = Z_VAR * var_se

        if case.family == "binomial":
            n = int(case.param1)
            p = mp.mpf(case.param2)
            lo_sum = int(mp.ceil(N_DRAWS * (mean - mean_tol)))
            hi_sum = int(mp.floor(N_DRAWS * (mean + mean_tol)))
            alpha_mean = exact_binom_two_sided_tail(N_DRAWS * n, p, lo_sum, hi_sum)
            chernoff_mean = chernoff_binom_two_sided_bound(N_DRAWS * n, p, lo_sum, hi_sum)
        else:
            lam = mp.mpf(case.param1)
            lo_sum = int(mp.ceil(N_DRAWS * (mean - mean_tol)))
            hi_sum = int(mp.floor(N_DRAWS * (mean + mean_tol)))
            alpha_mean = exact_poisson_two_sided_tail(N_DRAWS * lam, lo_sum, hi_sum)
            chernoff_mean = chernoff_poisson_two_sided_bound(N_DRAWS * lam, lo_sum, hi_sum)

        total_mean_fp += alpha_mean
        n_mean_checks += 1
        # Asymptotic normal tail for sample variance at z=5.6 is ~2.1e-8; budget 1.0e-7 each for finite-N margin
        alpha_var_asymp = mp.erfc(Z_VAR / mp.sqrt(2))
        total_var_budget += mp.mpf("1.0e-7")
        n_var_checks += 1

        print(
            f"\n[{case.name}] ({case.regime}) param1={case.param1} param2={case.param2}\n"
            f"  mean={fmt64(mean)} mean_tol={fmt64(mean_tol)} (exact_alpha={float(alpha_mean):.3e}, chernoff<={float(chernoff_mean):.3e})\n"
            f"  var={fmt64(var)} var_tol={fmt64(var_tol)} (asymp_alpha={float(alpha_var_asymp):.3e}, budgeted=1.000e-07)"
        )

        for k, cdf in zip(case.cutoffs, cdfs):
            cdf_se = mp.sqrt(cdf * (1 - cdf) / N_DRAWS)
            cdf_tol = Z_CDF * cdf_se
            lo_cnt = int(mp.ceil(N_DRAWS * (cdf - cdf_tol)))
            hi_cnt = int(mp.floor(N_DRAWS * (cdf + cdf_tol)))
            alpha_cdf = exact_binom_two_sided_tail(N_DRAWS, cdf, lo_cnt, hi_cnt)
            chernoff_cdf = chernoff_binom_two_sided_bound(N_DRAWS, cdf, lo_cnt, hi_cnt)
            total_cdf_fp += alpha_cdf
            n_cdf_checks += 1
            print(
                f"  k={k:3d}: cdf={fmt64(cdf)} cdf_tol={fmt64(cdf_tol)} "
                f"cnt_bounds=[{lo_cnt}, {hi_cnt}] exact_alpha={float(alpha_cdf):.3e} (chernoff<={float(chernoff_cdf):.3e})"
            )

    total_checks = n_cdf_checks + n_mean_checks + n_var_checks
    fwer_upper = total_cdf_fp + total_mean_fp + total_var_budget
    print("\n## 4. Family-Wise False-Positive Budget Summary")
    print(f"total_checks={total_checks} (cdf={n_cdf_checks}, mean={n_mean_checks}, var={n_var_checks})")
    print(f"sum_exact_cdf_alphas={float(total_cdf_fp):.4e}")
    print(f"sum_exact_mean_alphas={float(total_mean_fp):.4e}")
    print(f"sum_budgeted_var_alphas={float(total_var_budget):.4e}")
    print(f"fwer_bonferroni_upper_bound={float(fwer_upper):.4e} (stated budget <= 1.0e-5)")


if __name__ == "__main__":
    main()
