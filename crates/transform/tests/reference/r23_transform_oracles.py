#!/usr/bin/env python3
"""Independent Python 3.15 high-precision reference generator for CurveBook R23.1 & R23.3.

Uses 60-digit `decimal.Decimal` arithmetic (stdlib only) to compute arbitrary-precision
reference values for hypergeometric functions (`hyp1f1`, `hyp2f1`, `confluent_u`),
elementary/algebraic R23.1 functions, and exact R23.1 calculated-variable & window
semantics edge cases, then cross-checks against the R 4.5.2 JSON output from
`r23_transform_oracles.R` and writes the merged verified oracle artifact
`crates/transform/tests/reference/r23_transform_oracles.json`.

Run from repository root:
    /usr/local/bin/Rscript crates/transform/tests/reference/r23_transform_oracles.R
    /opt/homebrew/bin/python3 crates/transform/tests/reference/r23_transform_oracles.py
"""

from __future__ import annotations

import decimal
import json
import math
import pathlib
import sys
from decimal import Decimal

decimal.getcontext().prec = 60

D = Decimal


def d_exp(x: Decimal) -> Decimal:
    return x.exp()


def d_ln(x: Decimal) -> Decimal:
    return x.ln()


def d_hyp1f1(a: Decimal, b: Decimal, z: Decimal, max_terms: int = 500) -> Decimal:
    """Kummer confluent hypergeometric M(a, b, z) = 1F1(a; b; z) via 60-digit power series.

    Uses Kummer's first transformation M(a, b, z) = exp(z) * M(b - a, b, -z) when z < 0
    to avoid alternating-series cancellation (DLMF 13.2.39).
    """
    if z < 0:
        return d_exp(z) * d_hyp1f1(b - a, b, -z, max_terms=max_terms)
    term = D(1)
    total = term
    for k in range(1, max_terms + 1):
        term = term * (a + k - 1) / (b + k - 1) * z / k
        total += term
        if abs(term) < D("1e-55") * max(D(1), abs(total)):
            break
    return total


def d_hyp2f1_series(a: Decimal, b: Decimal, c: Decimal, z: Decimal, max_terms: int = 2000) -> Decimal:
    """Gauss hypergeometric 2F1(a, b; c; z) via 60-digit power series for |z| < 1."""
    term = D(1)
    total = term
    for k in range(1, max_terms + 1):
        term = term * (a + k - 1) * (b + k - 1) / ((c + k - 1) * k) * z
        total += term
        if abs(term) < D("1e-55") * max(D(1), abs(total)):
            break
    return total


def d_hyp2f1(a: Decimal, b: Decimal, c: Decimal, z: Decimal) -> Decimal:
    """Gauss hypergeometric 2F1(a, b; c; z) with Pfaff linear transformation for z < -0.5.

    DLMF 15.8.1: 2F1(a, b; c; z) = (1 - z)^(-a) * 2F1(a, c - b; c; z / (z - 1)).
    """
    if z < D("-0.5"):
        w = z / (z - D(1))
        return (D(1) - z) ** (-a) * d_hyp2f1_series(a, c - b, c, w)
    return d_hyp2f1_series(a, b, c, z)


def d_gamma_lanczos(x: Decimal) -> Decimal:
    """Gamma(x) at ~15-digit accuracy via math.gamma, refined for rational half-integers."""
    # Exact closed forms for positive integers and half-integers in 60-digit Decimal
    two_x = x * 2
    if two_x == int(two_x) and x > 0:
        n2 = int(two_x)
        if n2 % 2 == 0:
            return D(math.factorial(n2 // 2 - 1))
        # Gamma(k + 1/2) = sqrt(pi) * (2k)! / (4^k * k!)
        k = (n2 - 1) // 2
        pi_dec = D("3.14159265358979323846264338327950288419716939937510582097494459")
        return pi_dec.sqrt() * D(math.factorial(2 * k)) / (D(4) ** k * D(math.factorial(k)))
    # Fall back to standard double-precision math.gamma when non-half-integer
    return D(str(math.gamma(float(x))))


def d_confluent_u_nonint_b(a: Decimal, b: Decimal, z: Decimal) -> Decimal:
    """Tricomi U(a, b, z) via DLMF 13.2.42 connection formula for non-integer b."""
    # When a - b + 1 is a non-positive integer, 1 / Gamma(a - b + 1) = 0 so term1 vanishes
    # (for example U(a, a + 1, z) = z^(-a), DLMF 13.6.2).
    ab1 = a - b + D(1)
    if ab1 <= 0 and ab1 == int(ab1):
        term1 = D(0)
    else:
        g1 = D(str(math.gamma(float(D(1) - b))))
        g2 = D(str(math.gamma(float(ab1))))
        term1 = (g1 / g2) * d_hyp1f1(a, b, z)
    if a <= 0 and a == int(a):
        term2 = D(0)
    else:
        g3 = D(str(math.gamma(float(b - D(1)))))
        g4 = D(str(math.gamma(float(a))))
        term2 = (g3 / g4) * (z ** (D(1) - b)) * d_hyp1f1(ab1, D(2) - b, z)
    return term1 + term2


def build_special_function_oracles() -> dict:
    """High-precision hypergeometric and elementary R23.1 scalar oracles."""
    hyp1f1_cases = [
        ("M(1.5, 3.0, 1.2)", "1.5", "3.0", "1.2"),
        ("M(0.5, 1.5, -2.0)", "0.5", "1.5", "-2.0"),
        ("M(2.0, 5.0, 3.5)", "2.0", "5.0", "3.5"),
        ("M(1.25, 2.75, -0.8)", "1.25", "2.75", "-0.8"),
        ("M(-2.0, 3.0, 1.5)", "-2.0", "3.0", "1.5"),  # polynomial Laguerre case
        ("M(3.0, 2.0, 0.75)", "3.0", "2.0", "0.75"),  # a > b case
    ]
    hyp2f1_cases = [
        ("2F1(0.5, 1.0; 1.5; 0.25)", "0.5", "1.0", "1.5", "0.25"),
        ("2F1(1.5, 2.0; 3.5; 0.5)", "1.5", "2.0", "3.5", "0.5"),
        ("2F1(0.75, 1.25; 2.5; -0.6)", "0.75", "1.25", "2.5", "-0.6"),
        ("2F1(2.0, 1.5; 4.0; -2.0)", "2.0", "1.5", "4.0", "-2.0"),
        ("2F1(-3.0, 2.0; 1.5; 0.4)", "-3.0", "2.0", "1.5", "0.4"),  # terminating polynomial
        ("2F1(1.0, 1.0; 2.0; 0.5)", "1.0", "1.0", "2.0", "0.5"),    # exact: -ln(1-z)/z = 2*ln(2)
    ]
    # Non-integer b cases via connection formula; integer b = 2.0 verified via R integral
    u_cases_nonint = [
        ("U(0.5, 1.5, 2.5)", "0.5", "1.5", "2.5"),
        ("U(2.0, 1.25, 3.0)", "2.0", "1.25", "3.0"),
        ("U(1.25, 0.75, 0.5)", "1.25", "0.75", "0.5"),
        ("U(-2.0, 1.5, 2.0)", "-2.0", "1.5", "2.0"),  # polynomial case: z^2 - 3z + 1.875? check exact!
    ]
    # Exact polynomial U(-n, b, z) = (-1)^n * (b)_n * 1F1(-n; b; z) (DLMF 13.2.7)
    u_poly_m2 = D(1) * D("1.5") * D("2.5") * d_hyp1f1(D("-2.0"), D("1.5"), D("2.0"))

    hyp1f1_out = {label: float(d_hyp1f1(D(a), D(b), D(z))) for label, a, b, z in hyp1f1_cases}
    hyp2f1_out = {label: float(d_hyp2f1(D(a), D(b), D(c), D(z))) for label, a, b, c, z in hyp2f1_cases}
    u_out = {}
    for label, a, b, z in u_cases_nonint:
        if D(a) < 0:
            u_out[label] = float(u_poly_m2)
        else:
            u_out[label] = float(d_confluent_u_nonint_b(D(a), D(b), D(z)))

    # R23.1 locked scalar formula functions
    scalar_funcs = {
        # Locked: log(x) is base-10; ln(x) is natural log
        "log_base10_100": math.log10(100.0),
        "log_base10_0_001": math.log10(0.001),
        "log_base10_25": math.log10(25.0),
        "ln_exp2": math.log(math.exp(2.0)),
        # Locked: int(x) truncates toward zero (NOT floor)
        "int_trunc_3_7": float(math.trunc(3.7)),
        "int_trunc_neg3_7": float(math.trunc(-3.7)),
        "int_trunc_neg0_9": float(math.trunc(-0.9)),
        "int_trunc_0_0": float(math.trunc(0.0)),
        # Additional R23.1 scalar functions
        "fact_0": float(math.factorial(0)),
        "fact_7": float(math.factorial(7)),
        "fact_12": float(math.factorial(12)),
        "mod_17_5": math.fmod(17.0, 5.0),
        "mod_neg17_5_trunc_rem": math.fmod(-17.0, 5.0),
        "mod_neg17_5_euclid": (-17.0) % 5.0,
        "perc_12_5": 12.5 / 100.0,
        "asinh_2_5": math.asinh(2.5),
        "acosh_2_5": math.acosh(2.5),
        "atanh_0_6": math.atanh(0.6),
        "rad_180": math.radians(180.0),
        "rad_45": math.radians(45.0),
        "deg_pi_over_3": math.degrees(math.pi / 3.0),
        "pi_default": math.pi,
        "pi_times_2": 2.0 * math.pi,
    }

    return {
        "hyp1f1": hyp1f1_out,
        "hyp2f1": hyp2f1_out,
        "confluent_u_nonint_b": u_out,
        "scalar_r23_1_functions": scalar_funcs,
    }


def build_formula_window_edge_cases() -> dict:
    """Exact ground-truth table for locked R23.1 calculated-variable & window semantics.

    Dataset (8 rows):
      row | Group   | OrderKey | Value | MixedCol
      ----+---------+----------+-------+---------
       0  | "Ctrl"  | 20       | 10.0  | 10.0
       1  | "ctrl"  | 10       | 20.0  | "text"
       2  | "CTRL"  | 30       | 30.0  | None (Empty)
       3  | "Treat" | 10       | 40.0  | 40.0
       4  | "Treat" | 20       | 60.0  | "" (Blank string)
       5  | "Treat" | 30       | 80.0  | 80.0
       6  | None    | 10       | 50.0  | 50.0
       7  | ""      | 20       | 70.0  | "note"
    """
    rows = [
        {"row": 0, "group": "Ctrl",  "order": 20, "value": 10.0, "mixed": 10.0},
        {"row": 1, "group": "ctrl",  "order": 10, "value": 20.0, "mixed": "text"},
        {"row": 2, "group": "CTRL",  "order": 30, "value": 30.0, "mixed": None},
        {"row": 3, "group": "Treat", "order": 10, "value": 40.0, "mixed": 40.0},
        {"row": 4, "group": "Treat", "order": 20, "value": 60.0, "mixed": ""},
        {"row": 5, "group": "Treat", "order": 30, "value": 80.0, "mixed": 80.0},
        {"row": 6, "group": None,    "order": 10, "value": 50.0, "mixed": 50.0},
        {"row": 7, "group": "",      "order": 20, "value": 70.0, "mixed": "note"},
    ]

    def is_blank(v) -> bool:
        return v is None or v == ""

    def norm_key(v):
        if is_blank(v):
            return None
        return v.casefold() if isinstance(v, str) else v

    # 1. COUNT vs COUNTA on MixedCol
    # COUNT counts strictly numeric cells: rows 0, 3, 5, 6 -> 4
    # COUNTA counts all non-empty cells ("" treated as blank): rows 0, 1, 3, 5, 6, 7 -> 6
    count_numeric = sum(1 for r in rows if isinstance(r["mixed"], (int, float)))
    counta_nonempty = sum(1 for r in rows if not is_blank(r["mixed"]))

    # 2. 1-arg MIN(Value) / MAX(Value) column aggregate vs 2-arg MIN(Value, OrderKey) row-wise
    min_col_value = min(r["value"] for r in rows)
    max_col_value = max(r["value"] for r in rows)
    min_2arg_rowwise = [min(r["value"], float(r["order"])) for r in rows]
    max_2arg_rowwise = [max(r["value"], float(r["order"])) for r in rows]

    # 3. Case-insensitive string equality & blank "" semantics:
    #    Group == "ctrl" -> True for rows 0, 1, 2; False for 3, 4, 5; Empty (None) for blank 6, 7
    eq_ctrl_ci = [
        None if is_blank(r["group"]) else (r["group"].casefold() == "ctrl")
        for r in rows
    ]

    # 4. Partitioning with case-insensitive keys & blank partition key -> blank output:
    #    MEAN(Value) OVER (PARTITION BY Group ORDER BY OrderKey)  [NO ROWS clause -> FULL PARTITION]
    partitions: dict[str, list[dict]] = {}
    for r in rows:
        k = norm_key(r["group"])
        if k is not None:
            partitions.setdefault(k, []).append(r)

    mean_over_group_order_no_rows = []
    sum_over_group_order_no_rows = []
    for r in rows:
        k = norm_key(r["group"])
        if k is None:
            mean_over_group_order_no_rows.append(None)
            sum_over_group_order_no_rows.append(None)
        else:
            vals = [p["value"] for p in partitions[k]]
            mean_over_group_order_no_rows.append(sum(vals) / len(vals))
            sum_over_group_order_no_rows.append(sum(vals))

    # 5. Explicit ROWS 1 PRECEDING 1 FOLLOWING within partition ordered by OrderKey ASC
    #    (blank partition keys output None)
    mean_over_group_rows_1p_1f = [None] * len(rows)
    for k, members in partitions.items():
        ordered = sorted(members, key=lambda x: (x["order"], x["row"]))
        for idx, member in enumerate(ordered):
            lo = max(0, idx - 1)
            hi = min(len(ordered) - 1, idx + 1)
            window_vals = [ordered[j]["value"] for j in range(lo, hi + 1)]
            mean_over_group_rows_1p_1f[member["row"]] = sum(window_vals) / len(window_vals)

    # 6. WHERE + ROWS blanking:
    #    MEAN(Value) WHERE Value >= 20 OVER (PARTITION BY Group ORDER BY OrderKey ROWS 1 PRECEDING CURRENT ROW)
    #    Filtered-out rows (row 0 with Value = 10 < 20) and blank partition keys (rows 6, 7) MUST output None.
    mean_where_ge20_rows_1p_curr = [None] * len(rows)
    for k, members in partitions.items():
        active = sorted([m for m in members if m["value"] >= 20.0], key=lambda x: (x["order"], x["row"]))
        for idx, member in enumerate(active):
            lo = max(0, idx - 1)
            window_vals = [active[j]["value"] for j in range(lo, idx + 1)]
            mean_where_ge20_rows_1p_curr[member["row"]] = sum(window_vals) / len(window_vals)

    return {
        "dataset": rows,
        "count_mixed_numeric_only": count_numeric,
        "counta_mixed_non_empty": counta_nonempty,
        "min_1arg_value_col_agg": min_col_value,
        "max_1arg_value_col_agg": max_col_value,
        "min_2arg_value_order_rowwise": min_2arg_rowwise,
        "max_2arg_value_order_rowwise": max_2arg_rowwise,
        "group_eq_ctrl_case_insensitive": eq_ctrl_ci,
        "mean_value_over_group_orderby_no_rows_full_partition": mean_over_group_order_no_rows,
        "sum_value_over_group_orderby_no_rows_full_partition": sum_over_group_order_no_rows,
        "mean_value_over_group_orderby_rows_1prec_1foll": mean_over_group_rows_1p_1f,
        "mean_value_where_ge20_over_group_orderby_rows_1prec_curr": mean_where_ge20_rows_1p_curr,
    }


def main() -> None:
    ref_dir = pathlib.Path("crates/transform/tests/reference")
    r_json_path = ref_dir / "r23_transform_oracles_r.json"
    if not r_json_path.exists():
        raise SystemExit(f"Run r23_transform_oracles.R first; missing {r_json_path}")

    r_data = json.loads(r_json_path.read_text(encoding="utf-8"))
    py_special = build_special_function_oracles()
    py_windows = build_formula_window_edge_cases()

    # Cross-check Python 60-digit Decimal against R 4.5.2 numerical integration!
    r_spec = r_data["special_functions_r"]
    max_rel_err = 0.0
    for key, r_val in r_spec["hyp1f1_integral_crosscheck"].items():
        py_val = py_special["hyp1f1"][key]
        rel = abs(py_val - r_val) / max(1.0, abs(py_val))
        max_rel_err = max(max_rel_err, rel)
        assert rel < 1e-11, f"hyp1f1 mismatch on {key}: py={py_val}, R={r_val}, rel={rel}"
    for key, r_val in r_spec["hyp2f1_integral_crosscheck"].items():
        py_val = py_special["hyp2f1"][key]
        rel = abs(py_val - r_val) / max(1.0, abs(py_val))
        max_rel_err = max(max_rel_err, rel)
        assert rel < 1e-11, f"hyp2f1 mismatch on {key}: py={py_val}, R={r_val}, rel={rel}"
    for key, r_val in r_spec["confluent_u_integral_crosscheck"].items():
        if key in py_special["confluent_u_nonint_b"]:
            py_val = py_special["confluent_u_nonint_b"][key]
            rel = abs(py_val - r_val) / max(1.0, abs(py_val))
            max_rel_err = max(max_rel_err, rel)
            assert rel < 1e-11, f"confluent_u mismatch on {key}: py={py_val}, R={r_val}, rel={rel}"

    # Verify exact closed form 2F1(1, 1; 2; 0.5) = 2 * ln(2)
    closed_2f1 = 2.0 * math.log(2.0)
    assert abs(py_special["hyp2f1"]["2F1(1.0, 1.0; 2.0; 0.5)"] - closed_2f1) < 1e-15

    merged = {
        "provenance": {
            "r_script": "crates/transform/tests/reference/r23_transform_oracles.R",
            "python_script": "crates/transform/tests/reference/r23_transform_oracles.py",
            "r_version": r_data["r_version"],
            "python_version": sys.version.split()[0],
            "r_python_hypergeometric_max_rel_diff": max_rel_err,
        },
        "row_statistics": r_data["row_statistics"],
        "fraction_of_total": r_data["fraction_of_total"],
        "curve_calculus": r_data["curve_calculus"],
        "baseline_removal": r_data["baseline_removal"],
        "special_functions_r": r_spec,
        "special_functions_python_60digit": py_special,
        "formula_window_semantics": py_windows,
    }

    out_path = ref_dir / "r23_transform_oracles.json"
    out_path.write_text(json.dumps(merged, indent=2) + "\n", encoding="utf-8")
    r_json_path.unlink()
    print(f"Wrote {out_path} (R vs Python hypergeometric max rel diff = {max_rel_err:.3e})")


if __name__ == "__main__":
    main()
