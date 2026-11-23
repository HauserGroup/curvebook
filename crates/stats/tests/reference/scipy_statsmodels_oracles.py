"""Independent black-box SciPy 1.18.1 + statsmodels 0.15.0 D1 reference generator.

Run offline (once cached via uv):
    uv run --offline --python 3.13 --with scipy --with statsmodels \
        python crates/stats/tests/reference/scipy_statsmodels_oracles.py
"""

import numpy as np
import scipy
from scipy import optimize, stats
import statsmodels
from statsmodels.stats.diagnostic import lilliefors, normal_ad
from statsmodels.stats.inter_rater import cohens_kappa
from statsmodels.stats.multitest import multipletests

print(f"scipy={scipy.__version__} statsmodels={statsmodels.__version__} numpy={np.__version__}")

dp_norm = np.array([
    9.2, 9.5, 9.7, 9.8, 9.9, 10.0, 10.1, 10.1, 10.2, 10.2,
    10.3, 10.4, 10.4, 10.5, 10.6, 10.7, 10.8, 10.9, 11.1, 11.4,
])
dp_skew = np.array([
    1.1, 1.2, 1.2, 1.3, 1.4, 1.5, 1.5, 1.6, 1.7, 1.8,
    2.0, 2.1, 2.3, 2.6, 2.9, 3.3, 3.8, 4.5, 5.4, 6.8,
    8.5, 10.9, 14.2, 19.0, 26.5,
])

print("\n=== 1. D'Agostino-Pearson K^2 (scipy.stats.normaltest) ===")
for name, arr in [("norm", dp_norm), ("skew", dp_skew)]:
    res = stats.normaltest(arr)
    print(f"  {name}: K2={res.statistic:.15e} p={res.pvalue:.15e}")

print("\n=== 2. Anderson-Darling A2* (scipy.stats.anderson + statsmodels normal_ad) ===")
for name, arr in [("norm", dp_norm), ("skew", dp_skew)]:
    n = len(arr)
    ad_raw = stats.anderson(arr, dist="norm", method="interpolate").statistic
    ad_star = ad_raw * (1.0 + 0.75 / n + 2.25 / (n * n))
    sm_stat, sm_p = normal_ad(arr)
    print(f"  {name}: scipy_A2={ad_raw:.15e} scipy_A2*={ad_star:.15e} sm_A2={sm_stat:.15e} sm_p={sm_p:.15e}")

print("\n=== 3. Lilliefors KS (statsmodels.stats.diagnostic.lilliefors) ===")
dp_mid = np.array([
    1.0, 1.3, 1.5, 1.8, 2.0, 2.2, 2.5, 2.8, 3.1, 3.5,
    4.0, 4.6, 5.3, 6.2, 7.5, 9.2,
])
for name, arr in [("norm", dp_norm), ("mid", dp_mid), ("skew", dp_skew)]:
    d_stat, p_val = lilliefors(arr, dist="norm", pvalmethod="table")
    print(f"  {name}: D={d_stat:.15e} p_table={p_val:.15e}")

print("\n=== 4. Levene / Brown-Forsythe (scipy.stats.levene) ===")
g1 = [10.2, 10.8, 9.9, 11.1, 10.5, 10.3]
g2 = [12.1, 14.8, 11.2, 15.6, 13.4, 16.2, 12.9, 14.1]
g3 = [18.5, 22.0, 16.4, 24.1, 19.8]
g4 = [25.0, 25.4, 24.8, 25.7, 25.1, 25.5, 24.9]
for center in ["median", "mean"]:
    res = stats.levene(g1, g2, g3, g4, center=center)
    print(f"  het ({center}): F={res.statistic:.15e} p={res.pvalue:.15e}")

print("\n=== 5. Two-stage BKY FDR (statsmodels multipletests fdr_tsbky) ===")
raw_p = np.array([0.001, 0.004, 0.012, 0.031, 0.049, 0.120, 0.450, 0.890])
for alpha in [0.05, 0.10]:
    rej, qvals, _, _ = multipletests(raw_p, alpha=alpha, method="fdr_tsbky")
    print(f"  alpha={alpha}: reject={rej.tolist()}")
    print("    qvals=", [f"{q:.15e}" for q in qvals])

print("\n=== 6. Cohen's Kappa (statsmodels cohens_kappa: unweighted, linear, quadratic) ===")
table = np.array([
    [22,  5,  1],
    [ 3, 18,  4],
    [ 0,  2, 15],
])
for wt in [None, "linear", "quadratic"]:
    res = cohens_kappa(table, wt=wt, return_results=True)
    ci_lo, ci_hi = res.kappa - 1.959963984540054 * res.std_kappa, res.kappa + 1.959963984540054 * res.std_kappa
    print(f"  wt={str(wt):9s}: kappa={res.kappa:.15e} se={res.std_kappa:.15e} se0={res.std_kappa0:.15e} "
          f"z={res.z_value:.15e} p={res.pvalue_two_sided:.15e} ci=[{ci_lo:.15e}, {ci_hi:.15e}]")

print("\n=== 7. Noncentral-t & Noncentral-F effect-size CIs (scipy.stats.nct / ncf + brentq) ===")
def nct_ci(t_obs, df, conf=0.95):
    a = (1.0 - conf) / 2.0
    lo = optimize.brentq(lambda ncp: stats.nct.cdf(t_obs, df, ncp) - (1.0 - a), t_obs - 25.0, t_obs + 25.0, xtol=1e-13)
    hi = optimize.brentq(lambda ncp: stats.nct.cdf(t_obs, df, ncp) - a, t_obs - 25.0, t_obs + 25.0, xtol=1e-13)
    return lo, hi

ea = np.array([12.4, 13.1, 11.8, 12.9, 13.5, 12.2, 12.7])
eb = np.array([10.1, 10.8, 9.7, 11.0, 10.4, 10.2, 9.9, 10.6])
na, nb = len(ea), len(eb)
df_ab = na + nb - 2
sp = np.sqrt(((na - 1) * np.var(ea, ddof=1) + (nb - 1) * np.var(eb, ddof=1)) / df_ab)
t_ab = (np.mean(ea) - np.mean(eb)) / (sp * np.sqrt(1.0 / na + 1.0 / nb))
lo, hi = nct_ci(t_ab, df_ab)
scale = np.sqrt(1.0 / na + 1.0 / nb)
print(f"  unpaired d_ci=[{lo * scale:.15f}, {hi * scale:.15f}]")
