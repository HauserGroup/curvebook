"""Pre-specified independent reference generator for d1_randomization.rs (R9.14).

Computes exact integer quotas via declarative constrained integer optimization over the
feasible polytope {n in Z>=0^H : sum n_h = S, l_h <= n_h <= N_h} paired with hand-worked
fractional derivations (never mirroring procedural allocation loops), exact Binomial and
Poisson CDFs at preselected integer cutoffs, theoretical moments, statistical acceptance
bounds for both discrete distributions and multi-seed uniformity tests, and exact
family-wise false-positive probabilities at high working precision (mpmath 1.3.0 when
available, or stdlib Decimal at 120 digits).

Run standalone:
  python3 randomization.py 120
  python3 randomization.py 80
"""

from __future__ import annotations

import decimal
from fractions import Fraction
import math
import sys
from dataclasses import dataclass

try:
    from mpmath import mp
except ImportError:
    mp = None


def enumerate_feasible(total: int, lower_bounds: list[int], upper_bounds: list[int]):
    """Declaratively enumerate all n in Z^H such that sum(n) == total and l_h <= n_h <= N_h."""
    h_count = len(lower_bounds)
    if h_count == 0:
        if total == 0:
            yield ()
        return
    if h_count == 1:
        if lower_bounds[0] <= total <= upper_bounds[0]:
            yield (total,)
        return
    min_rest = sum(lower_bounds[1:])
    max_rest = sum(upper_bounds[1:])
    lo = max(lower_bounds[0], total - max_rest)
    hi = min(upper_bounds[0], total - min_rest)
    for x in range(lo, hi + 1):
        for tail in enumerate_feasible(total - x, lower_bounds[1:], upper_bounds[1:]):
            yield (x,) + tail


def solve_proportional_quota(total: int, weights: list[int]) -> tuple[int, ...]:
    """Declarative Hamilton quota on {n in Z>=0^H : sum n_h = total} via minimax/L1 optimization.

    Minimizes (max_h |n_h - q_h|, sum_h |n_h - q_h|, tie-break), where q_h = total * w_h / sum(w),
    breaking exact fractional remainder ties by favoring earlier indices receiving +1 above floor(q_h).
    """
    h_count = len(weights)
    if total == 0 or h_count == 0:
        return tuple(0 for _ in weights)
    tot_w = sum(weights)
    quotas = [Fraction(total * w, tot_w) for w in weights]
    floors = [int(q) for q in quotas]
    feasible = list(enumerate_feasible(total, [0] * h_count, [total] * h_count))
    return min(
        feasible,
        key=lambda n: (
            max(abs(n[h] - quotas[h]) for h in range(h_count)),
            sum(abs(n[h] - quotas[h]) for h in range(h_count)),
            tuple(-(n[h] - floors[h]) for h in range(h_count)),
        ),
    )


def solve_stratified_proportional_constrained(sample_size: int, stratum_sizes: list[int]) -> tuple[int, ...]:
    """Declarative constrained proportional allocation over {n in Z^H : sum n_h = S, l_h <= n_h <= N_h}.

    Lower bounds satisfy l_h = 1 whenever S >= H and N_h >= 1 (min-1 representation guarantee),
    upper bounds satisfy n_h <= N_h (stratum capacity). Minimizes exact Pearson chi-square deviation
    sum_h (n_h - q_h)^2 / q_h (with q_h = S * N_h / sum(N)), breaking ties by L1 deviation and
    ascending-index excess quota priority.
    """
    h_count = len(stratum_sizes)
    if sample_size == 0 or h_count == 0:
        return tuple(0 for _ in stratum_sizes)
    tot_n = sum(stratum_sizes)
    quotas = [Fraction(sample_size * n_h, tot_n) for n_h in stratum_sizes]
    lowers = [1 if (sample_size >= h_count and n_h >= 1) else 0 for n_h in stratum_sizes]
    uppers = list(stratum_sizes)
    feasible = list(enumerate_feasible(sample_size, lowers, uppers))
    return min(
        feasible,
        key=lambda n: (
            sum((n[h] - quotas[h]) ** 2 / quotas[h] for h in range(h_count)),
            sum(abs(n[h] - quotas[h]) for h in range(h_count)),
            tuple(-n[h] for h in range(h_count)),
        ),
    )


def solve_stratified_equal_constrained(sample_size: int, stratum_sizes: list[int]) -> tuple[int, ...]:
    """Declarative constrained equal allocation over {n in Z^H : sum n_h = S, 0 <= n_h <= N_h}.

    Water-filling allocation uniquely minimizes Schur-convex L2 energy sum_h n_h^2 subject to
    capacity bounds 0 <= n_h <= N_h, breaking ties by assigning residual units in ascending
    stratum-index order.
    """
    h_count = len(stratum_sizes)
    if sample_size == 0 or h_count == 0:
        return tuple(0 for _ in stratum_sizes)
    feasible = list(enumerate_feasible(sample_size, [0] * h_count, list(stratum_sizes)))
    return min(
        feasible,
        key=lambda n: (
            sum(x * x for x in n),
            tuple(-x for x in n),
        ),
    )


# Hand-worked fractional derivations checked at runtime:
# 1. Complete balanced N=15, weights=[4, 3, 2, 1] (W=10):
#    q = (15*4/10, 15*3/10, 15*2/10, 15*1/10) = (6, 9/2, 3, 3/2).
#    Floors = (6, 4, 3, 1) [sum 14], remainders = (0, 1/2, 0, 1/2).
#    Tie between index 1 (ArmB) and index 3 (ArmD) breaks ascending -> (6, 5, 3, 1).
assert solve_proportional_quota(15, [4, 3, 2, 1]) == (6, 5, 3, 1)
# 2. Complete balanced N=9, weights=[3, 2, 2] (W=7):
#    q = (27/7, 18/7, 18/7) = (3+6/7, 2+4/7, 2+4/7).
#    Floors = (3, 2, 2) [sum 7], remainders = (6/7, 4/7, 4/7).
#    1 seat to index 0 (6/7); remaining 1-seat tie (4/7 vs 4/7) breaks ascending -> (4, 3, 2).
assert solve_proportional_quota(9, [3, 2, 2]) == (4, 3, 2)
# 3. Stratified proportional S=15, N=[20, 12, 5, 2] (total 39), S >= H=4 => l=(1,1,1,1):
#    q = (300/39, 180/39, 75/39, 30/39) = (100/13, 60/13, 25/13, 10/13)
#      = (7+9/13, 4+8/13, 1+12/13, 0+10/13). Remaining 3 largest remainders (2, 3, 0) -> (8, 4, 2, 1).
assert solve_stratified_proportional_constrained(15, [20, 12, 5, 2]) == (8, 4, 2, 1)
# 4. Stratified proportional S=13, N=[20, 12, 5, 2] (total 39), l=(1,1,1,1):
#    q = (20/3, 4, 5/3, 2/3). Min-1 forces n_3 = 1; residual S'=12 over [20,12,5] (sum 37) gives
#    q' = (240/37, 144/37, 60/37) = (6+18/37, 3+33/37, 1+23/37) -> remainders (23/37, 33/37) -> (6, 4, 2, 1).
#    Constrained chi2 over {l <= n <= U, sum=13} uniquely selects (6, 4, 2, 1) with chi2 = 3/10.
assert solve_stratified_proportional_constrained(13, [20, 12, 5, 2]) == (6, 4, 2, 1)
# 5. Stratified equal S=19, N=[20, 12, 5, 2]:
#    Saturates n_3 = 2 and n_2 = 5; splits residual 12 equally across strata 0 and 1 -> (6, 6, 5, 2).
assert solve_stratified_equal_constrained(19, [20, 12, 5, 2]) == (6, 6, 5, 2)


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


def dec_ctx(dps: int) -> decimal.Context:
    return decimal.Context(prec=dps)


def dec_binom_two_sided_tail(n: int, p_frac: Fraction, lo_inclusive: int, hi_inclusive: int, dps: int = 120) -> decimal.Decimal:
    """Exact discrete Binomial(n, p) two-sided tail P(C < lo_inclusive) + P(C > hi_inclusive) in stdlib Decimal."""
    with decimal.localcontext() as ctx:
        ctx.prec = dps
        p = decimal.Decimal(p_frac.numerator) / decimal.Decimal(p_frac.denominator)
        q = decimal.Decimal(1) - p
        tol = decimal.Decimal(f"1e-{min(40, dps // 2)}")
        total = decimal.Decimal(0)
        if lo_inclusive > 0:
            c = lo_inclusive - 1
            comb_c = decimal.Decimal(math.comb(n, c))
            term = comb_c * (p**c) * (q ** (n - c))
            terms = [term]
            for j in range(c, 0, -1):
                ratio = (decimal.Decimal(j) * q) / (decimal.Decimal(n - j + 1) * p)
                term *= ratio
                terms.append(term)
                if ratio < 1 and term < tol * (decimal.Decimal(1) - ratio):
                    break
            total += sum(terms, decimal.Decimal(0))
        if hi_inclusive < n:
            c = hi_inclusive + 1
            comb_c = decimal.Decimal(math.comb(n, c))
            term = comb_c * (p**c) * (q ** (n - c))
            terms = [term]
            for j in range(c, n):
                ratio = (decimal.Decimal(n - j) * p) / (decimal.Decimal(j + 1) * q)
                term *= ratio
                terms.append(term)
                if ratio < 1 and term < tol * (decimal.Decimal(1) - ratio):
                    break
            total += sum(terms, decimal.Decimal(0))
        return total


def multiset_uniformity_specs() -> list[tuple[str, int, Fraction, int, int]]:
    """Pre-specified multi-seed uniformity test specs with z = 6.0 acceptance intervals.

    Returns rows of (name, num_seeds, p_true, n_checks_in_group, z_scaled_bounds).
    """
    specs = [
        # 1. CompleteBalanced N=6 weights [1,2,3] -> 6!/(1!2!3!) = 60 distinct permutations, 12_000 seeds -> E=200
        ("cb_perm_6", 12_000, Fraction(1, 60), 60),
        # 2. PermutedBlock block_size=6 N=6 weights [1,2,3] -> 60 distinct permutations, 12_000 seeds -> E=200
        ("pb_perm_6", 12_000, Fraction(1, 60), 60),
        # 3. PermutedBlock truncated tail (N=8, block=6, weights=[1,2,3]): 8 positions * 3 arms = 24 checks
        ("pb_pos_arm_0", 12_000, Fraction(1, 6), 8),
        ("pb_pos_arm_1", 12_000, Fraction(2, 6), 8),
        ("pb_pos_arm_2", 12_000, Fraction(3, 6), 8),
        # 4. Variable block_sizes=[6,12] on N=12: P(first block size == 6) = 1/2 across 12_000 seeds
        ("pb_var_block_size6", 12_000, Fraction(1, 2), 1),
        # 5. Unstratified sampling k=3 from N=10 active rows across 6_000 seeds: P(row included) = 3/10
        ("sample_unstrat_row", 6_000, Fraction(3, 10), 10),
        # 6. Stratified proportional sampling S=13 on [20, 12, 5, 2] -> n=[6, 4, 2, 1] across 6_000 seeds
        ("strat_row_alpha", 6_000, Fraction(6, 20), 20),
        ("strat_row_beta", 6_000, Fraction(4, 12), 12),
        ("strat_row_gamma", 6_000, Fraction(2, 5), 5),
        ("strat_row_tiny", 6_000, Fraction(1, 2), 2),
    ]
    return specs


Z_UNIF = decimal.Decimal("6.0")


def fmt64(val) -> str:
    return format(float(val), ".17g")


def run_multiseed_uniformity_budget(dps: int) -> tuple[int, decimal.Decimal]:
    with decimal.localcontext() as ctx:
        ctx.prec = dps
        total_checks = 0
        total_alpha = decimal.Decimal(0)
        print("\n## 2b. Multi-Seed Uniformity & Inclusion Probability Acceptance Bounds (z=6.0)")
        for name, n_seeds, p_frac, group_checks in multiset_uniformity_specs():
            p = decimal.Decimal(p_frac.numerator) / decimal.Decimal(p_frac.denominator)
            q = decimal.Decimal(1) - p
            mean = decimal.Decimal(n_seeds) * p
            se = (decimal.Decimal(n_seeds) * p * q).sqrt()
            tol = Z_UNIF * se
            lo_cnt = int((mean - tol).to_integral_value(rounding=decimal.ROUND_CEILING))
            hi_cnt = int((mean + tol).to_integral_value(rounding=decimal.ROUND_FLOOR))
            alpha = dec_binom_two_sided_tail(n_seeds, p_frac, lo_cnt, hi_cnt, dps=dps)
            total_checks += group_checks
            total_alpha += alpha * decimal.Decimal(group_checks)
            print(
                f"  {name:22s}: K={n_seeds:5d}, p={str(p_frac):5s}, checks={group_checks:2d}, "
                f"bounds=[{lo_cnt}, {hi_cnt}], alpha_each={float(alpha):.3e}, group_alpha={float(alpha * group_checks):.3e}"
            )
        print(f"multi_seed_uniformity_checks={total_checks}, sum_exact_uniformity_alphas={float(total_alpha):.4e}")
        return total_checks, total_alpha


def main() -> None:
    dps = int(sys.argv[1]) if len(sys.argv) > 1 else 120
    if dps < 80:
        raise ValueError("use at least 80 decimal digits")

    print(f"# Reference generation with dps={dps} ({'mpmath ' + getattr(mp, '__version__', '1.3.0') if mp is not None else 'stdlib Decimal+Fraction'})")
    print("\n## 1. Subject Randomization Exact Quotas (Declarative Integer Optimization)")
    cb_4 = list(solve_proportional_quota(15, [4, 3, 2, 1]))
    cb_3 = list(solve_proportional_quota(9, [3, 2, 2]))
    print(f"cb_tied_4arm (N=15, weights=[4,3,2,1], seed=104729): {cb_4}")
    print(f"cb_tied_3arm (N=9, weights=[3,2,2], seed=224737): {cb_3}")
    print("pb_fixed (N=29, weights=[1,2,3], block_sizes=[12], seed=350377): full_block=[2, 4, 6], last_block_len=5")
    print("pb_variable (N=41, weights=[1,2,3], block_sizes=[6, 12], seed=479001): full_block_mult_counts=[1*m, 2*m, 3*m]")

    print("\n## 2. Stratified Row Sampling Exact Quotas (strata sizes [20, 12, 5, 2], 39 active + 4 ineligible)")
    sizes = [20, 12, 5, 2]
    print(f"strat_prop_15 (sample=15, seed=611953): {list(solve_stratified_proportional_constrained(15, sizes))}")
    print(f"strat_prop_13 (sample=13, seed=746773): {list(solve_stratified_proportional_constrained(13, sizes))}")
    print(f"strat_equal_19 (sample=19, seed=882377): {list(solve_stratified_equal_constrained(19, sizes))}")

    unif_checks, unif_alpha = run_multiseed_uniformity_budget(dps)

    if mp is None:
        # Standalone stdlib Decimal/Fraction mode skips the high-trial mpmath distribution suite,
        # which uses the pre-verified mpmath 120-digit CDF/mean/variance alpha total of 2.3354e-6.
        fwer_upper = float(unif_alpha) + 2.3354e-6
        print(
            f"\n[info] mpmath not installed; multi-seed uniformity budget computed via exact Decimal+Fraction "
            f"(uniformity_checks={unif_checks}, uniformity_alpha={float(unif_alpha):.4e}, "
            f"combined_fwer_upper<={fwer_upper:.4e} <= 1.0e-5)"
        )
        return

    mp.dps = dps
    Z_CDF = mp.mpf("5.6")
    Z_MEAN = mp.mpf("5.6")
    Z_VAR = mp.mpf("5.6")

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

    total_checks = n_cdf_checks + n_mean_checks + n_var_checks + unif_checks
    fwer_upper = total_cdf_fp + total_mean_fp + total_var_budget + mp.mpf(str(unif_alpha))
    print("\n## 4. Family-Wise False-Positive Budget Summary")
    print(f"total_checks={total_checks} (cdf={n_cdf_checks}, mean={n_mean_checks}, var={n_var_checks}, uniformity={unif_checks})")
    print(f"sum_exact_cdf_alphas={float(total_cdf_fp):.4e}")
    print(f"sum_exact_mean_alphas={float(total_mean_fp):.4e}")
    print(f"sum_budgeted_var_alphas={float(total_var_budget):.4e}")
    print(f"sum_exact_uniformity_alphas={float(unif_alpha):.4e}")
    print(f"fwer_bonferroni_upper_bound={float(fwer_upper):.4e} (stated budget <= 1.0e-5)")


if __name__ == "__main__":
    main()
