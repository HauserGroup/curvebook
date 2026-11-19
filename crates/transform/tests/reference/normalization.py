"""Regenerate d1_normalization.rs reference values using exact rationals and mpmath 1.3.0.

Run with an isolated Python environment containing mpmath 1.3.0. The Rust tests
use the stored binary64 values offline. The optional CLI argument sets mpmath
working decimal precision (default 120, minimum 80) and cross-checks against an
80-digit evaluation.
"""

from fractions import Fraction
import sys

try:
    from mpmath import mp
except ImportError:
    mp = None

DPS_HIGH = int(sys.argv[1]) if len(sys.argv) > 1 else 120
if DPS_HIGH < 80:
    raise ValueError("use at least 80 decimal digits for nonlinear reference evaluations")
DPS_LOW = 80


def show(label, values):
    formatted = ", ".join(
        "None" if v is None else repr(float(v))
        for v in values
    )
    print(f"{label}: [{formatted}]")


def exact_frac(v):
    if v is None:
        return None
    if isinstance(v, Fraction):
        return v
    return Fraction(str(v))


def normalize_table():
    # Col 0 ("Ctrl"): [10, 30, 50, None, 90, 500*] -> active [10, 30, 50, 90]
    # Col 1 ("Drug"): [-200*, 30, 70, None, 110, 190] -> active [30, 70, 110, 190]
    cols = [
        [
            (Fraction(10), False),
            (Fraction(30), False),
            (Fraction(50), False),
            (None, False),
            (Fraction(90), False),
            (Fraction(500), True),
        ],
        [
            (Fraction(-200), True),
            (Fraction(30), False),
            (Fraction(70), False),
            (None, False),
            (Fraction(110), False),
            (Fraction(190), False),
        ],
    ]
    active_per_col = [[v for v, excl in col if v is not None and not excl] for col in cols]
    pooled_active = [v for col_vals in active_per_col for v in col_vals]

    print("=== Normalization (Fraction and Percent) ===")
    for per_data_set in (True, False):
        if per_data_set:
            anchors = [(min(vals), max(vals)) for vals in active_per_col]
        else:
            z_pool, h_pool = min(pooled_active), max(pooled_active)
            anchors = [(z_pool, h_pool) for _ in cols]

        for output_percent in (False, True):
            scale = Fraction(100) if output_percent else Fraction(1)
            mode_tag = f"per_ds={per_data_set}, pct={output_percent}"
            if per_data_set:
                for c, (z, h) in enumerate(anchors):
                    show(f"Norm {mode_tag} col{c} anchors", [z, h])
            else:
                show(f"Norm {mode_tag} pooled anchors", [anchors[0][0], anchors[0][1]])
            for c, (col, (z, h)) in enumerate(zip(cols, anchors)):
                out = [
                    None if v is None else (v - z) * scale / (h - z)
                    for v, _ in col
                ]
                show(f"Norm {mode_tag} col{c} values", out)

    # ControlColumnMean + InfoConstant + MeanSemN decreasing normalization
    ctrl_vals = [Fraction(10), Fraction(20), Fraction(30)]  # active mean = 20
    z_ctrl = sum(ctrl_vals) / len(ctrl_vals)
    h_hook = Fraction(100)
    sem_rows_in = [
        (Fraction(40), Fraction(4)),
        (Fraction(60), Fraction(8)),
        (Fraction(100), Fraction(4)),
    ]
    show("Norm MeanSemN hooked anchors", [z_ctrl, h_hook])
    for r, (m_in, s_in) in enumerate(sem_rows_in):
        show(
            f"Norm MeanSemN hooked pct row{r} [mean, sem]",
            [(m_in - z_ctrl) * 100 / (h_hook - z_ctrl), s_in * 100 / abs(h_hook - z_ctrl)],
        )
    # Decreasing anchors (zero=100, hundred=20) on MeanLowHigh [Mean=40, -Err=4, +Err=12]
    # and UpperLowerLimits [Mean=40, Lower=36, Upper=52] and partial-missing [40, 36, None]
    z_dec, h_dec = Fraction(100), Fraction(20)
    span_dec = abs(h_dec - z_dec)
    mean_in = Fraction(40)
    show(
        "Norm MeanLowHigh decreasing pct [mean, -err, +err]",
        [(mean_in - z_dec) * 100 / (h_dec - z_dec), Fraction(12) * 100 / span_dec, Fraction(4) * 100 / span_dec],
    )
    ul_m = (Fraction(40) - z_dec) * 100 / (h_dec - z_dec)
    ul_lo_from_hi = (Fraction(52) - z_dec) * 100 / (h_dec - z_dec)
    ul_hi_from_lo = (Fraction(36) - z_dec) * 100 / (h_dec - z_dec)
    show("Norm UpperLowerLimits decreasing pct [mean, lower, upper]", [ul_m, ul_lo_from_hi, ul_hi_from_lo])
    show("Norm UpperLowerLimits partial-missing decreasing pct [mean, lower, upper]", [ul_m, None, ul_hi_from_lo])


def baseline_removal():
    print("\n=== Baseline Removal (FirstAndLastLinear first_k=2, last_k=4, N=9) ===")
    row_count, first_k, last_k = 9, 2, 4

    cols = [
        # Col 0: trailing window has excluded 100* at row 6 -> active trailing rows (5, 7, 8), x_end=20/3
        [
            (Fraction(-4), False),
            (Fraction(-2), False),
            (Fraction(5), False),
            (Fraction(8), False),
            (None, False),
            (Fraction(6), False),
            (Fraction(100), True),
            (Fraction(9), False),
            (Fraction(12), False),
        ],
        # Col 1: leading window has missing row 1 -> active leading row (0,), x_start=0
        [
            (Fraction(1), False),
            (None, False),
            (Fraction(4), False),
            (Fraction(7), False),
            (Fraction(10), False),
            (Fraction(7), False),
            (Fraction(9), False),
            (Fraction(13), False),
            (Fraction(15), False),
        ],
        # Col 3: excluded 100* at row 6 with active trailing [6, 10, 12] -> x_end=20/3, y_end=28/3, B(r)=2r-4 -> B(2)=0
        [
            (Fraction(-4), False),
            (Fraction(-2), False),
            (Fraction(5), False),
            (Fraction(8), False),
            (None, False),
            (Fraction(6), False),
            (Fraction(100), True),
            (Fraction(10), False),
            (Fraction(12), False),
        ],
    ]

    for idx, col in enumerate(cols):
        c = idx if idx < 2 else 3
        first_pts = [(Fraction(r), v) for r, (v, excl) in enumerate(col[:first_k]) if v is not None and not excl]
        last_pts = [
            (Fraction(row_count - last_k + dr), v)
            for dr, (v, excl) in enumerate(col[row_count - last_k :])
            if v is not None and not excl
        ]
        x_start = sum(r for r, _ in first_pts) / len(first_pts)
        y_start = sum(v for _, v in first_pts) / len(first_pts)
        x_end = sum(r for r, _ in last_pts) / len(last_pts)
        y_end = sum(v for _, v in last_pts) / len(last_pts)
        span = x_end - x_start
        baselines = [
            Fraction(1, 2) * (y_start + y_end)
            if span == 0
            else y_start + (Fraction(r) - x_start) * (y_end - y_start) / span
            for r in range(row_count)
        ]
        show(f"Baseline Linear col{c} [x_start, y_start, x_end, y_end]", [x_start, y_start, x_end, y_end])
        show(f"Baseline Linear col{c} B(r)", baselines)
        diff = [None if v is None else v - b for (v, _), b in zip(col, baselines)]
        ratio = [None if (v is None or b == 0) else v / b for (v, _), b in zip(col, baselines)]
        pct = [None if (v is None or b == 0) else 100 * (v - b) / b for (v, _), b in zip(col, baselines)]
        frac = [None if (v is None or b == 0) else (v - b) / b for (v, _), b in zip(col, baselines)]
        show(f"Baseline Linear col{c} Difference", diff)
        show(f"Baseline Linear col{c} Ratio", ratio)
        show(f"Baseline Linear col{c} PercentChange", pct)
        show(f"Baseline Linear col{c} FractionalChange", frac)

    # Unbalanced replicate noiseless affine cancellation invariance (R = 8 rows, first_k = 3, last_k = 3)
    # y(r) = -5 + 3*r with blanks, exclusions, and replicate counts [1, 3, 2, 0, 2, 2, 1, 3]
    rep_rows = [
        (0, [Fraction(-5)], []),
        (1, [Fraction(-2), Fraction(-2), Fraction(-2)], [Fraction(999)]),
        (2, [Fraction(1), Fraction(1)], []),
        (3, [], []),
        (4, [Fraction(7), Fraction(7)], []),
        (5, [Fraction(10), Fraction(10)], [Fraction(500)]),
        (6, [Fraction(13)], []),
        (7, [Fraction(16), Fraction(16), Fraction(16)], []),
    ]
    first_obs = [(Fraction(r), v) for r, act, _ in rep_rows[:3] for v in act]
    last_obs = [(Fraction(r), v) for r, act, _ in rep_rows[5:] for v in act]
    ux_start = sum(r for r, _ in first_obs) / len(first_obs)
    uy_start = sum(v for _, v in first_obs) / len(first_obs)
    ux_end = sum(r for r, _ in last_obs) / len(last_obs)
    uy_end = sum(v for _, v in last_obs) / len(last_obs)
    assert ux_start == Fraction(7, 6) and uy_start == Fraction(-3, 2)
    assert ux_end == Fraction(37, 6) and uy_end == Fraction(27, 2)
    assert (uy_end - uy_start) / (ux_end - ux_start) == Fraction(3)
    print(
        f"Unbalanced Affine Anchors: x_start={ux_start} y_start={uy_start} x_end={ux_end} y_end={uy_end} slope={(uy_end - uy_start) / (ux_end - ux_start)}"
    )

    print("\n=== Baseline Removal (ColumnsRowWise on MeanSemN) ===")
    # Baseline col rows: 4, 0, None, 10*, -2; Treated rows: (10, 0.8), (5, 0.5), (12, 0.6), (20, 1.0), (7, 0.5)
    bases = [Fraction(4), Fraction(0), None, None, Fraction(-2)]
    treated = [
        (Fraction(10), Fraction(8, 10)),
        (Fraction(5), Fraction(5, 10)),
        (Fraction(12), Fraction(6, 10)),
        (Fraction(20), Fraction(1)),
        (Fraction(7), Fraction(5, 10)),
    ]
    for op in ("Difference", "Ratio", "PercentChange", "FractionalChange"):
        centers, sems = [], []
        for (m, s), b in zip(treated, bases):
            if b is None or (op != "Difference" and b == 0):
                centers.append(None)
                sems.append(None)
            elif op == "Difference":
                centers.append(m - b)
                sems.append(s)
            elif op == "Ratio":
                centers.append(m / b)
                sems.append(s / abs(b))
            elif op == "PercentChange":
                centers.append(100 * (m - b) / b)
                sems.append(100 * s / abs(b))
            elif op == "FractionalChange":
                centers.append((m - b) / b)
                sems.append(s / abs(b))
        show(f"Baseline RowWise {op} Mean", centers)
        show(f"Baseline RowWise {op} SEM", sems)

    # MeanLowHigh [Mean=8, -Err=1, +Err=3] with negative baseline b=-2
    b_neg = Fraction(-2)
    m_lh, lo_lh, hi_lh = Fraction(8), Fraction(1), Fraction(3)
    show("Baseline RowWise MeanLowHigh b=-2 Difference", [m_lh - b_neg, lo_lh, hi_lh])
    show("Baseline RowWise MeanLowHigh b=-2 Ratio", [m_lh / b_neg, hi_lh / abs(b_neg), lo_lh / abs(b_neg)])
    show(
        "Baseline RowWise MeanLowHigh b=-2 PercentChange",
        [100 * (m_lh - b_neg) / b_neg, 100 * hi_lh / abs(b_neg), 100 * lo_lh / abs(b_neg)],
    )
    show(
        "Baseline RowWise MeanLowHigh b=-2 FractionalChange",
        [(m_lh - b_neg) / b_neg, hi_lh / abs(b_neg), lo_lh / abs(b_neg)],
    )


def eval_transform_suite():
    sqrt2 = mp.sqrt(2)
    sqrt2pi = mp.sqrt(2 * mp.pi)
    ln10 = mp.log(10)

    def norm_ppf(p):
        return sqrt2 * mp.erfinv(2 * p - 1)

    transforms = {
        "Ln": (
            lambda u: mp.log(u),
            lambda u: 1 / u,
        ),
        "Log10": (
            lambda u: mp.log10(u),
            lambda u: 1 / (u * ln10),
        ),
        "Reciprocal": (
            lambda u: 1 / u,
            lambda u: -1 / (u * u),
        ),
        "NegLogM_uM": (
            lambda u: -mp.log10(u * mp.mpf("1e-6")),
            lambda u: -1 / (u * ln10),
        ),
        "Logit": (
            lambda u: mp.log(u / (1 - u)),
            lambda u: 1 / (u * (1 - u)),
        ),
        "Probit": (
            lambda u: 5 + norm_ppf(u),
            lambda u: sqrt2pi * mp.exp(mp.mpf("0.5") * norm_ppf(u) ** 2),
        ),
        "ArcsinSqrt": (
            lambda u: mp.asin(mp.sqrt(u)),
            lambda u: mp.mpf("0.5") / mp.sqrt(u * (1 - u)),
        ),
    }

    cases = [
        ("Ln", "2.5", "0.35"),
        ("Log10", "2.5", "0.35"),
        ("Reciprocal", "2.5", "0.35"),
        ("NegLogM_uM", "2.5", "0.35"),
        ("Logit", "0.25", "0.08"),
        ("Probit", "0.25", "0.08"),
        ("ArcsinSqrt", "0.25", "0.08"),
        ("Ln_half_ood_center", "0.5", "0.75"),
    ]

    results_by_dps = {}
    for dps in (DPS_LOW, DPS_HIGH):
        with mp.workdps(dps):
            res = {}
            for name, m_str, s_str in cases:
                fn_key = "Ln" if name == "Ln_half_ood_center" else name
                f, df = transforms[fn_key]
                m, s = mp.mpf(m_str), mp.mpf(s_str)
                center = f(m)
                delta_spread = abs(df(m)) * s
                if name == "Ln_half_ood_center":
                    lo_end, hi_end, ep_spread = None, None, None
                else:
                    lo_end = f(m - s)
                    hi_end = f(m + s)
                    ep_spread = mp.mpf("0.5") * abs(hi_end - lo_end)
                res[name] = (center, delta_spread, lo_end, hi_end, ep_spread)

            # UpperLowerLimits / MeanLowHigh decreasing monotone interval: [Mean=2.0, Lower=1.2, Upper=4.0]
            for fn_key in ("Reciprocal", "NegLogM_uM"):
                f, df = transforms[fn_key]
                m, lo, hi = mp.mpf("2.0"), mp.mpf("1.2"), mp.mpf("4.0")
                gm, glo, ghi = f(m), f(lo), f(hi)
                # Decreasing transform reverses ordering: ghi < gm < glo
                assert ghi < gm < glo
                mlh_ep_minus = gm - ghi
                mlh_ep_plus = glo - gm
                slope_abs = abs(df(m))
                mlh_delta_minus = (hi - m) * slope_abs
                mlh_delta_plus = (m - lo) * slope_abs
                res[f"Interval_{fn_key}"] = (gm, ghi, glo, mlh_ep_minus, mlh_ep_plus, mlh_delta_minus, mlh_delta_plus)

            # MeanCvN [Mean=2.5, %CV=14.0] -> SD=0.35 under Reciprocal (decreasing)
            m_cv, cv_in = mp.mpf("2.5"), mp.mpf("14.0")
            sd_cv = (cv_in / 100) * abs(m_cv)
            f_rec, df_rec = transforms["Reciprocal"]
            m_rec = f_rec(m_cv)
            cv_delta = 100 * (abs(df_rec(m_cv)) * sd_cv) / abs(m_rec)
            cv_ep = 100 * (mp.mpf("0.5") * abs(f_rec(m_cv + sd_cv) - f_rec(m_cv - sd_cv))) / abs(m_rec)
            res["MeanCvN_Reciprocal"] = (m_rec, cv_delta, cv_ep)
            results_by_dps[dps] = res

    # Cross-check 80 vs 120 digits
    max_dps_diff = mp.mpf(0)
    for key in results_by_dps[DPS_HIGH]:
        for a, b in zip(results_by_dps[DPS_LOW][key], results_by_dps[DPS_HIGH][key]):
            if a is not None and b is not None:
                max_dps_diff = max(max_dps_diff, abs(a - b))
    if max_dps_diff > mp.mpf("1e-70"):
        raise AssertionError(f"{DPS_LOW}-digit vs {DPS_HIGH}-digit mismatch: {max_dps_diff}")

    print(f"\n=== Math Transform Uncertainty Propagation (mpmath {DPS_HIGH} dps; max {DPS_LOW}-vs-{DPS_HIGH} diff = {float(max_dps_diff):.3e}) ===")
    for name, _, _ in cases:
        center, delta_spread, lo_end, hi_end, ep_spread = results_by_dps[DPS_HIGH][name]
        show(f"Math {name} [center, delta_spread, f(m-s), f(m+s), ep_spread]", [center, delta_spread, lo_end, hi_end, ep_spread])
    for fn_key in ("Reciprocal", "NegLogM_uM"):
        vals = results_by_dps[DPS_HIGH][f"Interval_{fn_key}"]
        show(f"Interval {fn_key} [center, lo=f(hi), hi=f(lo), mlh_ep(-), mlh_ep(+), mlh_delta(-), mlh_delta(+)]", vals)
    show("Math MeanCvN Reciprocal [center, delta_cv, ep_cv]", results_by_dps[DPS_HIGH]["MeanCvN_Reciprocal"])


def main():
    normalize_table()
    baseline_removal()
    if mp is not None:
        eval_transform_suite()
    else:
        print("\n[info] mpmath not installed; skipped nonlinear eval_transform_suite()")


if __name__ == "__main__":
    main()
