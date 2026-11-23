# SEC-08 Audit Report — Equation DSL & Model Library (`crates/fit`)

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095` (`main`)
- **Scope:** `SEC-08` (`crates/fit/src/dsl.rs`, `12` `.rs` files in `crates/fit/src/dsl/`, and `4` `.rs` files in `crates/fit/src/library/`: `17` files, `10,593` LOC total)
- **Date:** `2026-11-22`
- **Reviewer:** `rev-fit` (report-only static audit + read-only test verification; zero tracked files edited)
- **Summary Counts:** `S1: 0` · `S2: 4` · `S3: 5` · `S4: 3` (Total: `12` findings)
- **Prior Audit Check (`docs/audit/local-code-audit-2026-11-18.md`):**
  - Closed DSL/library findings (`#42`–`#50`, `#68`–`#69`, `#122`–`#126` via R4.6 & custom DSL tests): **0 regressions** observed in `dsl/**` or `library/**`.
  - Known open finding **`#127`** (default `"log_agonist_vs_response_4pl"` equation selection in `crates/ui-egui` and `crates/engine` without inspecting linear-vs-log X scale) concerns caller default selection outside `SEC-08` file scope and is not duplicated here.

## Baseline Verification Summary

- `cargo test --locked --offline -p curvebook-fit --lib`: exited `0` (`41 passed; 0 failed; 0 ignored`).
- `cargo test --locked --offline -p curvebook-fit --tests`: exited `0` (`227 passed; 0 failed; 0 ignored`: `41` unit tests in `src/lib.rs` + `186` integration tests across `27` integration test binaries; note `tests/nist_strd.rs` runs `1` test without `CURVEBOOK_REQUIRE_CORPUS=1` set).
- `wasm32` / architecture checks: workspace root `Cargo.toml` enforces `[workspace.lints.rust] unsafe_code = "forbid"` and Clippy `unwrap_used = "deny"`, `expect_used = "deny"`, `panic = "deny"`, inherited by `crates/fit/Cargo.toml` (`[lints] workspace = true`) alongside `crates/fit/src/lib.rs` (`#![forbid(unsafe_code)]`, `#![deny(missing_docs)]`); `cargo check --target wasm32-unknown-unknown` is blocked locally by missing `wasm32-unknown-unknown` sysroot (`E0463`), so `wasm32` cleanliness (zero `unsafe`, `std::fs`, `std::net`, `std::process`, or `SystemTime`; `std::thread_local!` only in `dsl/ode.rs`, which targets `wasm32`) was verified by source inspection and `rg` scan.

---

## File Coverage Table

| # | File | Lines | Inspected | Concrete Invariants & Dimensions Verified | Findings / Status |
|---|---|---:|---|---|---|
| 1 | `crates/fit/src/dsl.rs` | 53 | `1..=53` | Public DSL module facade & re-exports (`BinaryKind`, `BuiltinFn`, `DatasetFilter`, `DslCompileWarning`, `DslEquationKind`, `DslWarningKind`, `OpCode`, `UnaryKind`, `CompiledDslModel`, `integrate_rk45`), `D1` literature references (Dormand–Prince 1980, Hairer–Nørsett–Wanner 1993, Brent 1973, Abramowitz–Stegun 1964, DLMF Ch. 10/13/15, Thompson–Barnett 1986). | Clean — facade exports & `D1` citations verified |
| 2 | `crates/fit/src/dsl/ast.rs` | 521 | `1..=521` | `DatasetFilter::matches`, `Expr` iterative non-recursive `Clone`, `Debug`, and `Drop` (safe on 50k deep ASTs), `OpCode`, `BuiltinFn` variants, `DslEquationKind`, `DslCompileWarning`, `CompiledProgram`, `CompiledInitRule`. | Clean — iterative AST traits & type definitions verified |
| 3 | `crates/fit/src/dsl/compile.rs` | 337 | `1..=337` | Iterative `CompileTask` compiler, short-circuit `&&`/`||` & `IF` jump-target backpatching, `compile_ident` (`locals` before reserved symbols/predictors/params), `resolve_builtin_fn` arities (`1..=4`) & lowercase canonicalization, `check_opcode_budget` (always reports `{MAX_TOTAL_OPCODES}` even when called with `remaining`). | `FA-SEC08-12` |
| 4 | `crates/fit/src/dsl/implicit.rs` | 114 | `1..=114` | `implicit_probes_and_scale` multi-decade grid (`1e-30..1e30` + `x`/`params` multiples), `implicit_newton_fallback` (48 outer steps, central/one-sided FD, 16 step-halving inner steps, sign-change switch to `brent_scaled`, finiteness checks). | Clean — probe scaling & Newton fallback invariants verified |
| 5 | `crates/fit/src/dsl/lexer.rs` | 88 | `1..=88` | Pure-ASCII byte tokenizer, scientific notation with optional sign & rollback on stray `e`/`E`, `**` as `Caret`, `<>` as `NotEq`, `==`/`!=`/`<=`/`>=`/`&&`/`||`, UTF-8 slicing over ASCII spans only, invalid numeric/char `DslCompileError`. | Clean — tokenizer boundaries & error paths verified |
| 6 | `crates/fit/src/dsl/limits.rs` | 18 | `1..=18` | All 16 compile/VM/ODE resource caps (`MAX_VM_STEPS=100_000`, `MAX_ODE_STEPS=50_000`, `MAX_ODE_VM_OPS_PER_EVAL=5_000_000`, `MAX_PARSE_DEPTH=128`, `MAX_AST_NODES=4_096`, `MAX_DATASET_EXPANSION=10_000`, `MAX_DSL_SOURCE_BYTES=1_048_576`, `MAX_DSL_LINES=16_384`, `MAX_TOTAL_OPCODES=100_000`, `MAX_BESSEL_ORDER=100_000`, etc.). | Clean — bounded resource constants verified |
| 7 | `crates/fit/src/dsl/model.rs` | 653 | `1..=653` | `CompiledDslModel::compile`, source byte/line/dataset-index budgets, `[InitialValues]`/`[Transforms]`, `eval_implicit` (pole rejection via `verify_implicit_root`, tight `[0, root]` re-bracket, stale `\|F\| < 1e-7` doc), `eval_ode_single` + `OdeEvalBudget`, `initial_guess` unfiltered `mean_y` on direct calls, `derived_transforms`. | `FA-SEC08-08`, `FA-SEC08-10`, `FA-SEC08-12` |
| 8 | `crates/fit/src/dsl/ode.rs` | 660 | `1..=660` | Thread-local `OdeFitBudgetGuard` RAII restore, `CachedOdeTrajectory` LRU-4 cache keyed by `(model_id, ds_idx, t0, y0, params, x_tail)` bits, `thin_if_needed`, exact DOPRI5 Butcher tableau (`b_i`, `E_i` verified), `rk45_step`, `compute_ode_init_step_and_scale` (`hinit`), `advance_ode_frontier` (`DomainBoundary` vs `BudgetExceeded` vs `IntrinsicFailed`). | Clean — RK45 tableau, budgets & cache isolation verified |
| 9 | `crates/fit/src/dsl/parser.rs` | 334 | `1..=334` | Recursive-descent precedence (`&&`/`||`, `=`/`==`/`!=`/`<`/`<=`/`>`/`>=`, `+`/`-`, `*`/`/`/`%`, prefix `+`/`-`/`!`, right-assoc `^`), `MAX_PARSE_DEPTH` + `MAX_AST_NODES` enforcement (including chained unary/power), `IF`/`AND`/`OR`/`NOT` special forms, `LocalResolver::Scoped`. | Clean — precedence, node/depth bounds & AST construction verified |
| 10 | `crates/fit/src/dsl/special.rs` | 1121 | `1..=1121` | `Dd` 106-bit series for `J0/Y0/J1/Y1`, `hankel_pq` & `Dd` phase reduction (`|x| > 20`), Steed/Lentz CF1 + Wronskian normalization for `J_n(x)` & `Y_n(x)` upward recurrence (`|n| <= 100_000`), A&S 9.8 `I0/I1/K0/K1`, `hyp1f1` (Kummer + DLMF §13.7.2), `hyp2f1` (Pfaff + Gauss), `confluent_u` (polynomials + §13.7.3 + §13.2.9 integer-`b` + DLMF §13.4.4 Laplace Gauss–Kronrod + §13.3.7 downward `a`-recurrence). | Clean — special-function domains, cancellations & edge guards verified |
| 11 | `crates/fit/src/dsl/statements.rs` | 1192 | `1..=1192` | `strip_comment` delimiter order (`#` before `;`), `parse_dataset_prefix` range cap/dedup, `split_top_level_assignment`, `collect_filter_classes` equivalence partitioning, `resolve_equation_branches` (`explicit_outputs` / `outside_outputs` overwritten by `was_any`, mixed ODE branch IC validation), `is_builtin_constant_or_var` (`"t"` & `"X00"` handling), `compute_dataset_stats`, `compile_initial_rules`, `compile_transforms`. | `FA-SEC08-01`, `FA-SEC08-05`, `FA-SEC08-07`, `FA-SEC08-08`, `FA-SEC08-11` |
| 12 | `crates/fit/src/dsl/tests.rs` | 109 | `1..=109` | Private unit tests covering tiny/non-finite RK45 spans, `clamp`/Bessel extreme guards, dataset prefix overflow caps, `MAX_PARSE_DEPTH`, 20k/50k deep iterative AST `Clone`/`Debug`/`Drop` without stack overflow. | Clean — unit test assertions & stack safety verified |
| 13 | `crates/fit/src/dsl/vm.rs` | 319 | `1..=319` | `eval_bytecode_counted` (`MAX_VM_STEPS` termination, safe `ip` jump/bounds check, checked stack pops, `Div`/`Rem` zero rejection, `NaN` abort, intermediate `Inf` allowed for select overflow-saturating ops, final `out.is_finite()` filter), `eval_builtin` arity alignment with `compile.rs`. | Clean — VM bounds, div-by-zero, NaN & budget accounting verified |
| 14 | `crates/fit/src/library/curves.rs` | 1638 | `1..=1638` | All `43` curve models (`CURVE_SPECS`): parameter names, analytical `eval`/`grad`, `D1` literature citations on every struct, `WeibullPeak` `x == 0.0` discontinuity & missing `.is_finite()`, `GaussianCurve::initial_guess` unfiltered `max_by`. | `FA-SEC08-09`, `FA-SEC08-10` |
| 15 | `crates/fit/src/library/mod.rs` | 228 | `1..=228` | `ParametricModel` trait contract, `117`-equation (`47 + 43 + 27`) catalog/alias factory registry & drift test, `data_y_min_max` / `data_x_min_max` finite guards, `estimate_x_at_ymid` unfiltered `NaN`/`Inf` sort & interpolation. | `FA-SEC08-10` |
| 16 | `crates/fit/src/library/nist.rs` | 682 | `1..=682` | All `27` NIST StRD model specs (`NIST_SPECS`), `b1..bk` layouts, domain guards (`safe_exp`, denominators, `DanWood` `x > 0`, `Nelson` bivariate `x[1]`), exact analytical `grad` derivations across all `22` `NistKind` arms, `D1` references. | Clean — all 27 NIST formulas, gradients & guesses verified |
| 17 | `crates/fit/src/library/pharma.rs` | 2526 | `1..=2526` | All `47` pharma specs (`PHARMA_SPECS`), analytical `eval`/`grad` (`LogDoseResponse`, `LinearDoseResponse`, `OneSiteTotalAndNsGlobal`, `TwoSiteTotalAndNsGlobal`, `OneSiteSpecificLigandDepletion`, `OneSiteCompetitionLigandDepletion`, `AllostericModulatorSaturation`, `HomologousOneSiteCompetition`, `MotulskyMahanKinetics`), `D1` citations; `MorrisonKi` cancellation, `OneSiteBinding` (`with_hill`) `NaN` overflow/underflow, `RichardsFivePl::derived_transforms` `+Inf` overflow in `default_bounds`, `AssocThenDissoc` unguarded `(-kobs * t0).exp()` & `arg > 700` NaN checks, `estimate_global_ns_and_bg` unfiltered `ds1`. | `FA-SEC08-02`, `FA-SEC08-03`, `FA-SEC08-04`, `FA-SEC08-06`, `FA-SEC08-10` |

---

## Findings (`S1` -> `S4`)

### `FA-SEC08-01` · `S2` · Prefixed DSL equation branches (`<A>`, `<~A>`) are silently overwritten by a subsequent unprefixed fallback equation (`wrong-result`)

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/fit/src/dsl/statements.rs:854-915`
- **Category:** `wrong-result`
- **Defect:** In `resolve_equation_branches`, ODE initial-condition targets explicitly enforce specificity so that dataset-scoped `<A>`/`<~A>` rules take precedence over unprefixed `DatasetFilter::Any` defaults regardless of source line order (`explicit_inits` checks `if is_specific || !prev_specific` at lines `859–862`, and `outside_inits` uses `.find(|&&(_, _, spec)| spec)` at line `906`). By contrast, main equation targets overwrite `explicit_outputs[c_idx]` unconditionally (lines `866–868`) and scan `outside_outputs.into_iter().rev()` without preferring `!was_any` (lines `891–915`).
- **Failure scenario:** Any valid multi-dataset DSL equation that declares a specific dataset branch before an unprefixed default — e.g., `<A> Y = Baseline \n Y = Bottom + (Top - Bottom) / (1 + 10^(LogEC50 - X))` or `<~A> Y = a * X \n Y = b * X` — has the `<A>` or `<~A>` target silently overwritten by the trailing `Any` branch, dropping any parameters exclusive to the specific branch and fitting the wrong equation on datasets `A` or `~A` without any compile error or warning.
- **Evidence:**
```rust
// crates/fit/src/dsl/statements.rs:854-869
            } else if is_ode_init {
                let is_specific = !was_any;
                if c_idx == outside_class {
                    outside_inits.push((class_locals[outside_class].len(), item_idx, is_specific));
                } else {
                    let prev_specific = explicit_inits[c_idx].is_some_and(|(_, _, s)| s);
                    if is_specific || !prev_specific {
                        explicit_inits[c_idx] = Some((class_locals[c_idx].len(), item_idx, is_specific));
                    }
                }
            } else if c_idx == outside_class {
                outside_outputs.push((class_locals[outside_class].len(), item_idx, was_any));
            } else {
                explicit_outputs[c_idx] = Some((class_locals[c_idx].len(), item_idx));
            }
```
```rust
// crates/fit/src/dsl/statements.rs:891-915
    for (loc_len, item_idx, was_any) in outside_outputs.into_iter().rev() {
        ...
        pending_branches.push(PendingBranch { ... });
        break;
    }
```
- **Suggested fix:** Track `(loc_len, item_idx, is_specific)` in `explicit_outputs` using the same `if is_specific || !prev_specific` rule as `explicit_inits`, and select specific (`!was_any`) entries ahead of `was_any` fallback entries in `outside_outputs` (matching `outside_inits` at line `906`) after preserving last-writer-wins within each specificity tier.
- **Related:** `FA-SEC08-08`.

---

### `FA-SEC08-02` · `S2` · `MorrisonKi::eval` suffers catastrophic cancellation returning uninhibited velocity `Vo` in low-`Et` / high-inhibitor regimes (`wrong-result`)

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/fit/src/library/pharma.rs:1743-1753`
- **Category:** `wrong-result`
- **Defect:** `MorrisonKi::eval` (`morrison_tight_binding_ki`) computes the Morrison (1969) tight-binding quadratic via `(sum * sum - 4.0 * et * i_conc).max(0.0).sqrt()` and `1.0 - (sum - disc) / (2.0 * et)` without discriminant reformulation or rationalization.
- **Failure scenario:** When enzyme concentration `Et` is small compared to `sum = Et + I + Ki_app` (classic nanomolar/picomolar enzyme assays or excess inhibitor `I >> Et`), `4 * Et * I` is dwarfed by `sum^2`: intermediate regimes (`Et = 1e-12, Ki = 1e-10, S = 1e-3, Km = 1e-3, I = 1e-5`) lose ~7 digits in `sum - disc` (corrupting finite-difference gradients in LM), and when `4 * Et * I / sum^2 < f64::EPSILON / 2` (e.g. `Et = 1e-18, Ki = 1e-3, S = 0, I = 0.1`, where true velocity is `0.009901 * Vo`), `sum * sum - 4.0 * et * i_conc` rounds identically to `sum * sum`, `disc == sum`, `sum - disc == 0.0`, and `eval` returns `Vo` (0% inhibition). Contrast `OneSiteSpecificLigandDepletion::solve_free_ligand` (`pharma.rs:1988-1994`), which rationalizes the quadratic root cleanly.
- **Evidence:**
```rust
// crates/fit/src/library/pharma.rs:1749-1753
        let ki_app = ki * (1.0 + s_conc / km);
        let sum = et + i_conc + ki_app;
        let disc = (sum * sum - 4.0 * et * i_conc).max(0.0).sqrt();
        Some(vo * (1.0 - (sum - disc) / (2.0 * et)))
```
- **Suggested fix:** Compute `disc = (et - i_conc + ki_app).hypot(2.0 * (i_conc * ki_app).sqrt())` (non-negative and overflow-safe), and evaluate the free-enzyme fraction cancellation-free as `(2.0 * ki_app) / ((i_conc + ki_app - et) + disc)` when `i_conc + ki_app >= et`, or `((et - i_conc - ki_app) + disc) / (2.0 * et)` otherwise (with final `.is_finite()` check).
- **Related:** `crates/fit/src/library/pharma.rs:1988-1994`.

---

### `FA-SEC08-03` · `S2` · `OneSiteBinding::eval` (`with_hill`) returns `Some(f64::NAN)` inside `default_bounds` due to unnormalized `powf` (`wrong-result`)

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/fit/src/library/pharma.rs:1166-1174` (and `1591-1596`)
- **Category:** `wrong-result`
- **Defect:** `OneSiteBinding::eval` for `one_site_specific_hill` (`with_hill == true`, lines `1171–1173`) computes `c.powf(h)` and `kd.powf(h)` separately and returns `Some(bmax * ch / (kdh + ch))` without normalizing by `(c / kd).powf(h)` or guarding finiteness. `AllostericSigmoidal::eval` (`pharma.rs:1591-1596`) also computes `s.powf(h)` and `khalf.powf(h)` independently (avoiding `NaN` only by failing with `None`).
- **Failure scenario:** Within `OneSiteBinding::default_bounds()` (`h in [0.05, 20.0]`, `kd >= 1e-30`, lines `1210–1212`), valid finite inputs such as `c = 1e20, kd = 1e20, h = 20.0` (both `powf` terms overflow to `+Inf`, yielding `+Inf / +Inf`) or `c = 1e-20, kd = 1e-20, h = 20.0` (both underflow to `0.0`, yielding `0.0 / 0.0`) or `c = 1e30, kd = 1.0, h = 20.0` return `Some(f64::NAN)`, violating `ParametricModel::eval`'s finite-or-`None` contract. Compare `LinearDoseResponse::eval` (`pharma.rs:682-687`), which avoids both failure modes using `ratio = (c / c50).powf(hill)` and saturating on `!ratio.is_finite()`.
- **Evidence:**
```rust
// crates/fit/src/library/pharma.rs:1166-1174
        if self.with_hill {
            let h = params[2];
            if c == 0.0 {
                return Some(0.0);
            }
            let ch = c.powf(h);
            let kdh = kd.powf(h);
            return Some(bmax * ch / (kdh + ch));
        }
```
```rust
// crates/fit/src/library/pharma.rs:1591-1596
        let sh = s.powf(h);
        let kh = khalf.powf(h);
        if !sh.is_finite() || !kh.is_finite() || (kh + sh) == 0.0 {
            return None;
        }
        Some(vmax * sh / (kh + sh))
```
- **Suggested fix:** Replace separate `c.powf(h)` / `kd.powf(h)` calls in both `OneSiteBinding` and `AllostericSigmoidal` with `ratio = (c / kd).powf(h)`, handling `ratio == 0.0` (`0.0`), `!ratio.is_finite()` (when `ratio.is_infinite()`, saturating to `bmax`/`vmax`, provided `h > 0`), and checking `.is_finite()` on the final `Some(...)` value.
- **Related:** `crates/fit/src/library/pharma.rs:682-687`.

---

### `FA-SEC08-04` · `S2` · `RichardsFivePl::derived_transforms` overflows `2^(1/S)` and emits `-Inf`/`+Inf` `LogEC50` inside `default_bounds` (`wrong-result`)

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/fit/src/library/pharma.rs:790-804`
- **Category:** `wrong-result`
- **Defect:** `RichardsFivePl::derived_transforms` computes `((2.0_f64.powf(1.0 / s) - 1.0).log10()) / hill` directly without handling exponent overflow for small `S` or underflow for large `S`, and pushes `DerivedScalar { name: "LogEC50", value: log_ec50 }` without verifying `log_ec50.is_finite()`.
- **Failure scenario:** `RichardsFivePl::default_bounds()` permits `S` down to `1e-4` (line `791`). For any fitted `S < 1.0 / 1024.0 ≈ 9.77e-4` (spanning `[1e-4, 9.76e-4]` inside the model's own bounds), `1.0 / s >= 1024.0`, so `2.0_f64.powf(1.0 / s)` overflows to `+Inf`, `(Inf - 1.0).log10()` evaluates to `+Inf`, and `derived_transforms` emits a non-finite `LogEC50 = -Inf` (or `+Inf` for negative `HillSlope`). If bounds are relaxed and `S > 1e16`, `2^(1/S) - 1` rounds to `0.0` and also emits non-finite `LogEC50`.
- **Evidence:**
```rust
// crates/fit/src/library/pharma.rs:790-804
    fn default_bounds(&self) -> Vec<(Option<f64>, Option<f64>)> {
        vec![(None, None), (None, None), (None, None), (None, None), (Some(1e-4), Some(100.0))]
    }
    fn derived_transforms(&self, params: &[f64]) -> Vec<DerivedScalar> {
        let (log_xb, hill, s) = (params[2], params[3], params[4]);
        let mut out = Vec::new();
        if s > 0.0 && hill.abs() > 1e-12 {
            let log_ec50 = log_xb - ((2.0_f64.powf(1.0 / s) - 1.0).log10()) / hill;
            out.push(DerivedScalar { name: "LogEC50".to_owned(), value: log_ec50, units: None });
            if let Some(ec50) = safe_pow10(log_ec50) {
                out.push(DerivedScalar { name: "EC50".to_owned(), value: ec50, units: None });
            }
        }
        out
    }
```
- **Suggested fix:** Compute `log10(2^(1/s) - 1)` overflow- and underflow-safely: for `1/s > 100.0`, use `core::f64::consts::LOG10_2 / s + (-core::f64::consts::LN_2 / s).exp().neg().ln_1p() / core::f64::consts::LN_10`; for small `1/s`, use `(core::f64::consts::LN_2 / s).exp_m1().log10()`; and push `LogEC50` only when `log_ec50.is_finite()`.
- **Related:** None.

---

### `FA-SEC08-05` · `S3` · Unbound or out-of-scope lowercase local variable `t` silently compiles as predictor `X` without error or warning (`wrong-result`)

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/fit/src/dsl/statements.rs:314-324, 502-523, 600-641, 744, 1149-1192`
- **Category:** `wrong-result`
- **Defect:** `build_eq_items` (`statements.rs:314-316`) intentionally allows lowercase `t` as a local variable assignment (`is_builtin_constant_or_var(&name) && name != "t"`), adding `"t"` to `all_local_names` and `scoped_local_names`. However, `is_builtin_constant_or_var("t")` (`1150-1152`) folds `"t"` to `"T"` and returns `true`, so `collect_free_idents` (`1170`) never reports an unbound `"t"` to `validate_branch_locals_and_trailing` or `branch_uses_unbound_scoped_indices`.
- **Failure scenario:** Two invalid usages fail to trigger compile errors and instead silently substitute predictor `X`:
  1. Forward reference before assignment (`u = t * k \n t = X^2 \n Y = u`): `"t"` is absent from `free` at line `599` (never caught as `"referenced before assignment"`), absent from `local_map` when compiling `u`, so `compile_ident` (`compile.rs:247`) resolves `"t"` to `LoadPredictor(0)` (`X`), while `collect_dsl_warnings` (`statements.rs:744`) suppresses the reserved-`t` warning because `"t"` is in `local_names`.
  2. Cross-branch out-of-scope reference (`<A> t = 2 * X \n <A> Y = a * t \n <B> Y = b * t`): `<B>` omits local `t`, skips the out-of-scope check on line `634` because `"t"` is not in `free`, and silently evaluates `<B>` with `t = X` and no warning.
- **Evidence:**
```rust
// crates/fit/src/dsl/statements.rs:314-324, 1149-1175
                } else if is_builtin_constant_or_var(&name) && name != "t" {
                    // Uppercase `T` and all other built-in symbols (`X`, `E`, `PI`, `DS`, `INF`, `NAN`, ...)
                    // cannot be assigned as locals, whereas lowercase `t` may act as an explicit `[Equation]` temporary.
                    ...
                } else {
                    items.push(EqItem::Local { filter, name, rhs, line_no });
                }
...
pub(in crate::dsl) fn is_builtin_constant_or_var(name: &str) -> bool {
    let u = name.to_ascii_uppercase();
    if matches!(u.as_str(), "X" | "T" | "Y" | "DS" | "DATASET" | "PI" | "E" | "LN2" | "LN10" | "SQRT2" | "INF" | "NAN") {
        return true;
    }
...
pub(in crate::dsl) fn collect_free_idents(...) {
...
                if !is_builtin_constant_or_var(name) && !bound_locals.contains(name) && !seen.contains(name) {
```
- **Suggested fix:** Check `branch_local_names.contains(name.as_str())` and `scoped_names.contains(name)` directly in `validate_branch_locals_and_trailing` / `branch_uses_unbound_scoped_indices` (or check non-builtin + declared-local names via a separate local-dependency AST walker) before applying `is_builtin_constant_or_var`, so any forward or out-of-scope use of `"t"` raises the intended compile error instead of falling back to `LoadPredictor(0)`.
- **Related:** `crates/fit/src/dsl/compile.rs:241-247`.

---

### `FA-SEC08-06` · `S3` · `AssocThenDissoc::eval` omits `arg > 700.0` guard on `(-kobs * t0).exp()` and kinetic binding `eval`s allow `NaN` propagation (`wrong-result`)

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/fit/src/library/pharma.rs:1351-1354, 1387-1390, 1424-1428, 1466-1477`
- **Category:** `wrong-result`
- **Defect:** While `AssocThenDissoc::eval` guards `(-kobs * t) > 700.0` (`1466`) and `(-koff * (t - t0)) > 700.0` (`1473`), line `1471` evaluates `(-kobs * t0).exp()` without checking `> 700.0` (and `AssocThenDissoc` has no `default_bounds()`). In addition, `DissociationOnePhase` (`1351`), `AssociationOneConc` (`1387`), `AssociationTwoConc` (`1424`), and `AssocThenDissoc` (`1466`, `1473`) only test `arg > 700.0` rather than `!arg.is_finite() || arg > 700.0` or using `safe_exp`.
- **Failure scenario:** If `AssocThenDissoc` is evaluated at a negative `Time0` with large `kobs` (`-kobs * t0 > 709.78`, e.g., during unconstrained LM steps), `(-kobs * t0).exp()` overflows to `+Inf`, making `bound_at_t0 = -Inf` and returning `Some(f64::NEG_INFINITY)` near `t = t0` or `Some(f64::NAN)` (`-Inf * 0.0`) at large `t`; direct callers passing `f64::NAN` or `0.0 * f64::INFINITY` into `arg > 700.0` similarly receive `Some(f64::NAN)` across all four kinetic models instead of `None`.
- **Evidence:**
```rust
// crates/fit/src/library/pharma.rs:1464-1477
        if t < t0 {
            let arg = -kobs * t;
            if arg > 700.0 {
                return None;
            }
            Some(eq * (1.0 - arg.exp()))
        } else {
            let bound_at_t0 = eq * (1.0 - (-kobs * t0).exp());
            let arg = -koff * (t - t0);
            if arg > 700.0 {
                return None;
            }
            Some(bound_at_t0 * arg.exp())
        }
```
- **Suggested fix:** Replace manual `arg > 700.0` checks across `DissociationOnePhase`, `AssociationOneConc`, `AssociationTwoConc`, and `AssocThenDissoc` (including line `1471`) with a `safe_exp` helper checking `!arg.is_finite() || arg > 700.0` and check `.is_finite()` on the returned value; also consider `default_bounds()` (`Kon >= 0`, `Koff >= 0`, `LigandConc >= 0`, `Time0 >= 0` if appropriate).
- **Related:** None.

---

### `FA-SEC08-07` · `S3` · `strip_comment` checks `#` before `;` instead of choosing the earliest comment delimiter in the line (`wrong-result`)

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/fit/src/dsl/statements.rs:27-54`
- **Category:** `wrong-result`
- **Defect:** `strip_comment` loops `for marker in ['#', ';']` and immediately returns at the first found marker in iteration order (`#` before `;`), rather than locating whichever of `'#'` or `';'` occurs earliest in the line.
- **Failure scenario:** Any DSL line using a `;` comment whose text happens to include `#` (e.g. `Y = A * X ; sample #1`, `; note #2`, or `Y = A * X ; disabled # initial: A=5`) splits at `#` instead of `;`, returning `"Y = A * X ; sample"` or `"; note"` as executable statement text and failing compilation with `Unexpected character ';'`.
- **Evidence:**
```rust
// crates/fit/src/dsl/statements.rs:27-54
pub(in crate::dsl) fn strip_comment<'a>(line: &'a str, line_no: usize, hints: &mut HashMap<String, f64>) -> Result<&'a str, FitError> {
    let trimmed = line.trim();
    for marker in ['#', ';'] {
        if let Some(pos) = trimmed.find(marker) {
            let comment = trimmed[pos + 1..].trim();
            ...
            return Ok(trimmed[..pos].trim());
        }
    }
    Ok(trimmed)
}
```
- **Suggested fix:** Select `let pos = [trimmed.find('#'), trimmed.find(';')].into_iter().flatten().min();` so comment stripping always starts at the leftmost `#` or `;` on the line.
- **Related:** None.

---

### `FA-SEC08-08` · `S3` · Multi-branch ODE models omit per-branch initial-condition validation when another branch provides `Y0` (`contract-drift`)

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/fit/src/dsl/statements.rs:942-949` and `crates/fit/src/dsl/model.rs:194-197, 233-273`
- **Category:** `contract-drift`
- **Defect:** `resolve_equation_branches` fails compilation only when `has_ode && has_ode_init && ode_init_branches.is_empty()` (`statements.rs:942`), and `CompiledDslModel::compile` synthesizes the default `Y0` parameter only when `ode_init_branches.is_empty()` (`model.rs:194, 234`). Neither checks that every compiled ODE branch (`body_stmts`) has a matching entry in `ode_init_branches`.
- **Failure scenario:** Given `<A> Y0 = yA \n <A> dY/dX = -k1 * Y \n <B> dY/dX = -k2 * Y`, `<A>` populates `ode_init_branches`, so compilation succeeds cleanly without compiling a default `Y0` or flagging `<B>`. `default_ode_init_idx` stays `None` (`model.rs:232, 260-269`), so every evaluation of dataset `<B>` fails at runtime in `select_ode_init` (`model.rs:456`) rather than failing with a descriptive `DslCompileError`.
- **Evidence:**
```rust
// crates/fit/src/dsl/statements.rs:932-949
    for b in pending_branches {
        if let Some(target) = target_for_resolved_kind(&eq_items[b.item_idx], final_kind) {
            if has_ode && let Some((init_loc_len, init_idx)) = b.init_sel {
                let (t0, init_locals, init_rhs) =
                    resolve_ode_init_for_class(&eq_items, &class_locals[b.class_idx][..init_loc_len], init_idx, &all_local_names)?;
                ode_init_branches.push((b.filter.clone(), t0, init_locals, init_rhs));
            }
            body_stmts.push((b.filter, b.locals, target));
        }
    }
    if has_ode && has_ode_init && ode_init_branches.is_empty() {
        return Err(FitError::DslCompileError { ... });
    }
```
```rust
// crates/fit/src/dsl/model.rs:194-197
        if kind == DslEquationKind::OdeFirstOrder && ode_init_branches.is_empty() && !seen_params.contains("Y0") {
            param_order.push("Y0".to_owned());
            seen_params.insert("Y0".to_owned());
        }
```
- **Suggested fix:** When `has_ode` and `has_ode_init` (or `ode_init_branches` is non-empty), verify that every branch in `pending_branches` has `b.init_sel.is_some()` (or explicitly fall back to fitted parameter `Y0` at `t0 = 0.0` for branches without an explicit initial rule) and otherwise return `FitError::DslCompileError`.
- **Related:** `FA-SEC08-01`.

---

### `FA-SEC08-09` · `S3` · `WeibullPeak::eval` hardcodes `0.0` at `X = 0` for `Shape <= 1` and omits `.is_finite()` on overflow (`wrong-result`)

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/fit/src/library/curves.rs:980-992`
- **Category:** `wrong-result`
- **Defect:** `WeibullPeak::eval` returns `Some(0.0)` whenever `x0 == 0.0` regardless of `Shape` (`params[2]`), and returns `Some(...)` on the final product `amp * (shape / scale) * u.powf(shape - 1.0) * e` without checking `.is_finite()` (`991`).
- **Failure scenario:** At `Shape == 1.0` (the Exponential limit `f(x) = (Amplitude / Scale) * exp(-x / Scale)`), `eval(&[0.0], &[100.0, 2.0, 1.0], 0)` returns `Some(0.0)` while `eval(&[1e-12], &[100.0, 2.0, 1.0], 0)` returns `Some(50.0)`, creating a spurious jump discontinuity at `X = 0`. For `0.0 < Shape < 1.0`, the density has a `+Inf` pole at `x0 -> 0+`, yet `eval` returns `0.0` at `x0 == 0.0` and can overflow `u.powf(shape - 1.0)` to `Some(f64::INFINITY)` for very small `x0 > 0`.
- **Evidence:**
```rust
// crates/fit/src/library/curves.rs:980-992
    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let x0 = x.first().copied()?;
        let (amp, scale, shape) = (params[0], params[1], params[2]);
        if x0 < 0.0 || scale <= 0.0 || shape <= 0.0 {
            return None;
        }
        if x0 == 0.0 {
            return Some(0.0);
        }
        let u = x0 / scale;
        let e = safe_exp(-u.powf(shape))?;
        Some(amp * (shape / scale) * u.powf(shape - 1.0) * e)
    }
```
- **Suggested fix:** Handle `x0 == 0.0` by returning `Some(0.0)` for `shape > 1.0`, `Some(amp / scale)` (if finite) for `(shape - 1.0).abs() <= 1e-15`, and `None` for `shape < 1.0` (singularity at `0`); wrap line `991` in `.is_finite().then_some(...)`.
- **Related:** None.

---

### `FA-SEC08-10` · `S4` · Several `ParametricModel::initial_guess` helpers omit finite-point filtering for direct trait callers (`no-panic`)

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/fit/src/library/mod.rs:158-179`, `crates/fit/src/library/curves.rs:809`, `crates/fit/src/library/pharma.rs:1771-1779`, and `crates/fit/src/dsl/model.rs:585`
- **Category:** `no-panic`
- **Defect:** Unlike `data_y_min_max`, `data_x_min_max`, `compute_dataset_stats`, and `estimate_centered_linear_seed`, four `initial_guess` helpers consume unfiltered `points`: `estimate_x_at_ymid` (`library/mod.rs:164-178`), `GaussianCurve::initial_guess` (`curves.rs:809`), `estimate_global_ns_and_bg` (`pharma.rs:1771-1779`), and `CompiledDslModel::initial_guess` (`dsl/model.rs:585`).
- **Failure scenario:** While the nonlinear solver pipeline (`solver/layout.rs`) strips non-finite observations before calling `initial_guess`, direct callers of the public `ParametricModel::initial_guess` trait passing a `DataPoint` with `NaN` or `±Inf` receive `NaN` seeds (`total_cmp` ranks `f64::NAN` above `+Inf` in `GaussianCurve`, and `estimate_x_at_ymid` propagates `NaN` into midpoint interpolation across dose-response and binding models) or clamped `0.1` fallback in `CompiledDslModel` (`f64::NAN.max(0.1) == 0.1`).
- **Evidence:**
```rust
// crates/fit/src/library/mod.rs:164-173
    let mut sorted: Vec<(f64, f64)> = points.iter().map(|p| (p.x0(), p.y)).collect();
    sorted.sort_by(|a, b| a.0.total_cmp(&b.0));
...
                return x0 + frac * (x1 - x0);
```
```rust
// crates/fit/src/library/curves.rs:809
        let peak_x = points.iter().max_by(|a, b| a.y.total_cmp(&b.y)).map_or(0.5 * (xmin + xmax), |p| p.x0());
```
```rust
// crates/fit/src/dsl/model.rs:585
        let mean_y = if points.is_empty() { 1.0 } else { points.iter().map(|p| p.y).sum::<f64>() / points.len() as f64 };
```
- **Suggested fix:** Filter `points.iter().filter(|p| p.x0().is_finite() && p.y.is_finite())` in `estimate_x_at_ymid`, `GaussianCurve::initial_guess`, `estimate_global_ns_and_bg`, and `CompiledDslModel::initial_guess` (matching `compute_dataset_stats`).
- **Related:** `crates/fit/src/library/mod.rs:132-155`.

---

### `FA-SEC08-11` · `S4` · Predictor-index parsing diverges between `is_builtin_constant_or_var` and `compile_ident` on multi-zero / overflow `X<digits>` tokens (`contract-drift`)

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/fit/src/dsl/statements.rs:1154-1160` and `crates/fit/src/dsl/compile.rs:258-266`
- **Category:** `contract-drift`
- **Defect:** `is_builtin_constant_or_var` classifies any `X<digits>` other than literal `"0"` (`rest != "0" && rest.bytes().all(|b| b.is_ascii_digit())`) as a built-in predictor variable, whereas `compile_ident` additionally requires `rest.parse::<usize>()` with `idx >= 1`.
- **Failure scenario:** Identifiers such as `X00` or an integer-overflowing `X9999999999999999999999` are skipped by `collect_free_idents` (so they are omitted from `param_names`) and then fail codegen lookup with `"Unknown symbol X00"` rather than either compiling consistently as a user parameter or emitting an explicit invalid-predictor-index diagnostic.
- **Evidence:**
```rust
// crates/fit/src/dsl/statements.rs:1154-1160
    if let Some(rest) = u.strip_prefix('X')
        && !rest.is_empty()
        && rest != "0"
        && rest.bytes().all(|b| b.is_ascii_digit())
    {
        return true;
    }
```
```rust
// crates/fit/src/dsl/compile.rs:258-266
            if let Some(rest) = u.strip_prefix('X')
                && !rest.is_empty()
                && rest != "0"
                && let Ok(idx) = rest.parse::<usize>()
                && idx >= 1
            {
                code.push(OpCode::LoadPredictor(idx - 1));
                return Ok(());
            }
```
- **Suggested fix:** Share one helper `parse_predictor_index(name: &str) -> Option<usize>` (`rest.parse::<usize>().ok().filter(|&idx| idx >= 1).map(|idx| idx - 1)`) between `statements.rs` (`is_builtin_constant_or_var` / `reserved_ident_info`) and `compile.rs` (`compile_ident`).
- **Related:** `FA-SEC08-05`.

---

### `FA-SEC08-12` · `S4` · Stale `eval_implicit` tolerance doc comment and hardcoded opcode-budget diagnostic (`cleanup`)

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/fit/src/dsl/model.rs:321-325` and `crates/fit/src/dsl/compile.rs:14-23`
- **Category:** `cleanup`
- **Defect:** `CompiledDslModel::eval_implicit`'s doc comment (`model.rs:323-324`) still states `"accept only |F| < 1e-7"`, whereas the implementation uses scale-invariant relative tolerances (`1e-11 * ref_f` / step tolerance in `implicit.rs:69, 107` and `verify_implicit_root` in `model.rs:374-375`). Separately, `check_opcode_budget` (`compile.rs:17`) hardcodes `{MAX_TOTAL_OPCODES}` in its error message even when called with a smaller per-branch `remaining` budget.
- **Failure scenario:** Maintainers reading `eval_implicit`'s docstring can misjudge root residual acceptance behavior, and a multi-branch DSL exceeding the cumulative `100_000` opcode budget reports `"(100000 opcodes)"` on the offending branch without indicating it was the remaining cumulative budget.
- **Evidence:**
```rust
// crates/fit/src/dsl/model.rs:323-324
    /// back to a step-clamped Newton iteration with central-difference derivative and accept
    /// only `|F| < 1e-7`. Only one root is returned (the first bracket found), so equations with
```
```rust
// crates/fit/src/dsl/compile.rs:14-20
pub(in crate::dsl) fn check_opcode_budget(code: &[OpCode], max_opcodes: usize) -> Result<(), FitError> {
    if code.len() > max_opcodes {
        return Err(FitError::DslCompileError {
            message: format!("Compiled equation exceeds total bytecode budget ({MAX_TOTAL_OPCODES} opcodes)"),
            line: 1,
            column: 1,
        });
    }
```
- **Suggested fix:** Update `eval_implicit`'s doc comment to describe `verify_implicit_root` and `implicit_newton_fallback`'s scale-invariant tolerances accurately, and mention `max_opcodes` remaining vs `MAX_TOTAL_OPCODES` in `check_opcode_budget`.
- **Related:** None.
