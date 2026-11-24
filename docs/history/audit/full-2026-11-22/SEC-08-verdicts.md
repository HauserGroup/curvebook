# SEC-08 Verification Verdicts — Equation DSL & Model Library (`crates/fit`)

- **Base Checked:** `7c85a8e516165ddadb101236a88eec937bf1f095` (`main`)
- **Verifier:** `ver-backend-3` (independent source inspection + solver/result pipeline trace + isolated `.lc-worktrees/ver-backend-3` dynamic probe `ver_sec08_probe`, removed after execution)
- **Source Report:** [SEC-08.md](SEC-08.md) (`FA-SEC08-01`..`FA-SEC08-12`)

## Summary Table

| Verdict | Count | Settled Severity Breakdown |
|---|---:|---|
| `CONFIRMED` | 12 | `S1: 0`, `S2: 4`, `S3: 5`, `S4: 3` |
| `PLAUSIBLE` | 0 | `—` |
| `REFUTED` | 0 | `—` |
| `DUPLICATE` | 0 | `—` |
| **Total** | **12** | **`S1=0, S2=4, S3=5, S4=3`** |

Dynamic verification run against an isolated detached worktree at `7c85a8e516165ddadb101236a88eec937bf1f095` (probe test removed afterward; worktree clean via `git status -s`):
- Clippy check (`CARGO_TARGET_DIR=/tmp/cb-audit-ver-backend-3 cargo clippy --manifest-path .lc-worktrees/ver-backend-3/Cargo.toml --locked --offline -p curvebook-fit --all-targets -- -D warnings`): exited `0` with zero warnings.
- Baseline suite (`CARGO_TARGET_DIR=/tmp/cb-audit-ver-backend-3 cargo test --manifest-path .lc-worktrees/ver-backend-3/Cargo.toml --locked --offline -p curvebook-fit --quiet`): exited `0` with `228 passed; 0 failed; 0 ignored` (`41` unit tests + `186` integration tests across `27` test binaries + `1` doc-test; note `tests/nist_strd.rs` runs `1` test without `CURVEBOOK_REQUIRE_CORPUS=1`).
- Targeted reproducer suite (`CARGO_TARGET_DIR=/tmp/cb-audit-ver-backend-3 cargo test --manifest-path .lc-worktrees/ver-backend-3/Cargo.toml --locked --offline -p curvebook-fit --test ver_sec08_probe -- --nocapture`): `running 11 tests` -> `test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.04s`.

Precision notes recorded during verification (without refuting any defect):
- `FA-SEC08-10`: `estimate_x_at_ymid` returns early if an earlier finite window already spans `y_mid` (`[xy(1.0, 10.0), xy(2.0, 20.0), xy(NaN, 30.0)]` avoids touching row 2), whereas `[xy(1.0, 10.0), xy(NaN, 30.0)]` returns `f64::NAN`. Furthermore, non-finite table cells are filtered during `extract_datasets_from_sheet`, whereas direct `fit_nonlinear` callers enter `ParameterLayout::build` (`solver/layout.rs#L65-L70`) *before* `build_observations`, where `clamp_to_bounds` (`solver/layout.rs#L311`) silently sanitizes non-finite initial guesses to `0.0` clamped to bounds.

---

## File Coverage Completeness Check

Verified `100%` file coverage against `SEC-08` scope at `7c85a8e516165ddadb101236a88eec937bf1f095` (`17` files, `10,593` total lines across `crates/fit/src/dsl.rs`, `crates/fit/src/dsl/**`, and `crates/fit/src/library/**`, zero omitted files):

- `crates/fit/src/`: [dsl.rs](../../../../crates/fit/src/dsl.rs) (`53`)
- `crates/fit/src/dsl/`: [ast.rs](../../../../crates/fit/src/dsl/ast.rs) (`521`), [compile.rs](../../../../crates/fit/src/dsl/compile.rs) (`337`), [implicit.rs](../../../../crates/fit/src/dsl/implicit.rs) (`114`), [lexer.rs](../../../../crates/fit/src/dsl/lexer.rs) (`88`), [limits.rs](../../../../crates/fit/src/dsl/limits.rs) (`18`), [model.rs](../../../../crates/fit/src/dsl/model.rs) (`653`), [ode.rs](../../../../crates/fit/src/dsl/ode.rs) (`660`), [parser.rs](../../../../crates/fit/src/dsl/parser.rs) (`334`), [special.rs](../../../../crates/fit/src/dsl/special.rs) (`1,121`), [statements.rs](../../../../crates/fit/src/dsl/statements.rs) (`1,192`), [tests.rs](../../../../crates/fit/src/dsl/tests.rs) (`109`), [vm.rs](../../../../crates/fit/src/dsl/vm.rs) (`319`)
- `crates/fit/src/library/`: [curves.rs](../../../../crates/fit/src/library/curves.rs) (`1,638`), [mod.rs](../../../../crates/fit/src/library/mod.rs) (`228`), [nist.rs](../../../../crates/fit/src/library/nist.rs) (`682`), [pharma.rs](../../../../crates/fit/src/library/pharma.rs) (`2,526`)

Every file and line count matches `SEC-08.md`'s File Coverage Table with zero omissions.

---

## Per-Finding Verdicts

### Verdict `FA-SEC08-01` · `CONFIRMED`

- **Verifier:** `ver-backend-3`
- **Evidence:** In [resolve_equation_branches](../../../../crates/fit/src/dsl/statements.rs#L854-L915), ODE initial-condition slots (`explicit_inits`, `outside_inits`) explicitly prioritize `!was_any` specificity, whereas main equation targets overwrite `explicit_outputs[c_idx]` unconditionally (`#L866-L868`) and walk `outside_outputs.into_iter().rev()` (`#L891-L915`), selecting the trailing `was_any` fallback ahead of earlier `<A>` or `<~A>` rules. Dynamic `probe_01_dsl_prefixed_branch_overwritten_by_trailing_fallback` confirmed both failure paths: `<A> Y = Baseline \n Y = Bottom + (Top - Bottom) / (1 + 10^(LogEC50 - X))` dropped `"Baseline"` completely from `param_names()` (`["Bottom", "Top", "LogEC50"]`) and evaluated the 4PL fallback for dataset `0` (`60.0` at `X=0`); similarly `<~A> Y = a * X \n Y = b * X` dropped `"a"` and evaluated `b * X` on dataset `1`. Reversing the line order restores `<A>`/`<~A>` evaluation.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (silently replaces dataset-scoped DSL model branches with the fallback equation, visibly dropping branch-specific parameters or changing fitted curve predictions).
- **Still present on main:** yes

### Verdict `FA-SEC08-02` · `CONFIRMED`

- **Verifier:** `ver-backend-3`
- **Evidence:** [MorrisonKi::eval](../../../../crates/fit/src/library/pharma.rs#L1743-L1753) computes `(sum * sum - 4.0 * et * i_conc).max(0.0).sqrt()` and `1.0 - (sum - disc) / (2.0 * et)` without discriminant rationalization (contrast [OneSiteSpecificLigandDepletion::solve_free_ligand](../../../../crates/fit/src/library/pharma.rs#L1988-L1994)). Dynamic `probe_02_morrison_ki_catastrophic_cancellation` confirmed that at `[Vo=100, Et=1e-18, Ki=1e-3, S=0, Km=1e-3], I=0.1` (`4 * Et * I / sum^2 < f64::EPSILON/2`), `eval` returns exact `100.0` (`Vo`, 0% inhibition instead of `~0.990099`), and in nanomolar/picomolar regime `(Et=1e-12, Ki=1e-10, S=1e-3, Km=1e-3, I=1e-5)` loses >7 digits of accuracy (`rel_err > 1e-7` against the cancellation-free `hypot` + rationalized expression).
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (severe cancellation returning uninhibited `Vo` / degraded finite-difference gradients on valid low-`Et` / excess-inhibitor inputs).
- **Still present on main:** yes

### Verdict `FA-SEC08-03` · `CONFIRMED`

- **Verifier:** `ver-backend-3`
- **Evidence:** Inside [OneSiteBinding::eval](../../../../crates/fit/src/library/pharma.rs#L1166-L1174), the `with_hill` branch evaluates `c.powf(h)` and `kd.powf(h)` separately and returns `Some(bmax * ch / (kdh + ch))` without normalizing by `(c / kd).powf(h)` or checking `.is_finite()`, while [AllostericSigmoidal::eval](../../../../crates/fit/src/library/pharma.rs#L1591-L1596) returns `None` instead of saturating. Dynamic `probe_03_one_site_hill_returns_nan_inside_default_bounds` confirmed that all three within-bounds parameterizations (`default_bounds` = `[(Some(0.0), None), (Some(1e-30), None), (Some(0.05), Some(20.0))]`) — `(c=1e20, kd=1e20, h=20)`, `(1e-20, 1e-20, 20)`, and `(1e30, 1.0, 20)` — return `Some(v)` where `v.is_nan()`, violating [ParametricModel::eval](../../../../crates/fit/src/library/mod.rs#L37-L39)'s finite-or-`None` contract (whereas `allosteric_sigmoidal_kinetics` returns `None`).
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (violates `ParametricModel::eval` finite contract by returning `Some(NaN)` within model `default_bounds` at extreme scale inputs).
- **Still present on main:** yes

### Verdict `FA-SEC08-04` · `CONFIRMED`

- **Verifier:** `ver-backend-3`
- **Evidence:** [RichardsFivePl::default_bounds](../../../../crates/fit/src/library/pharma.rs#L790-L792) permits `S >= 1e-4`, and [RichardsFivePl::derived_transforms](../../../../crates/fit/src/library/pharma.rs#L793-L804) computes `2.0_f64.powf(1.0 / s)` directly without finiteness filtering, then [fit_nonlinear](../../../../crates/fit/src/solver/fit.rs#L146), [assemble_dataset_results](../../../../crates/fit/src/solver/assembly.rs#L126), and [result.rs](../../../../crates/fit/src/result.rs#L745-L754) push every `DerivedScalar` into `ResultSheet.scalars` without filtering non-finite values. Dynamic `probe_04_richards_5pl_derived_transforms_inf_inside_bounds` confirmed `derived_transforms` returns `LogEC50 = -Inf` (for `hill = 1.0`) and `+Inf` (for `hill = -1.0`) at `S = 1e-4`, and an end-to-end `execute_analysis` call on a `log_agonist_vs_response_5pl` sheet with `S = 1e-4` emitted a non-finite `LogEC50` entry in `result.scalars`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (emits `-Inf`/`+Inf` derived `LogEC50` scalars within default parameter bounds into `ResultSheet` outputs).
- **Still present on main:** yes

### Verdict `FA-SEC08-05` · `CONFIRMED`

- **Verifier:** `ver-backend-3`
- **Evidence:** [build_eq_items](../../../../crates/fit/src/dsl/statements.rs#L314-L324) permits lowercase `"t"` as a local variable (`all_local_names`, `scoped_local_names`), while [is_builtin_constant_or_var](../../../../crates/fit/src/dsl/statements.rs#L1149-L1162) uppercases `"t"` -> `"T"` and returns `true`, causing [collect_free_idents](../../../../crates/fit/src/dsl/statements.rs#L1164-L1192) to skip `"t"` during [validate_branch_locals_and_trailing](../../../../crates/fit/src/dsl/statements.rs#L595-L642) and [branch_uses_unbound_scoped_indices](../../../../crates/fit/src/dsl/statements.rs#L502-L523). Codegen in [compile_ident](../../../../crates/fit/src/dsl/compile.rs#L245-L248) falls back to `OpCode::LoadPredictor(0)` (`X`), and [collect_dsl_warnings](../../../../crates/fit/src/dsl/statements.rs#L743-L746) suppresses warnings because `"t"` is in `all_local_names`. Dynamic `probe_05_lowercase_local_t_unbound_silently_uses_x` confirmed both `u = t * k \n t = X^2 \n Y = u` and `<A> t = 2 * X \n <A> Y = a * t \n <B> Y = b * t` compile with zero errors and empty `warnings()`, silently substituting `X` (`7*3 = 21.0` and `5*3 = 15.0`).
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` (invalid forward/out-of-scope use of local `t` silently resolves to `X`; workaround is naming temporaries something other than `t`).
- **Still present on main:** yes

### Verdict `FA-SEC08-06` · `CONFIRMED`

- **Verifier:** `ver-backend-3`
- **Evidence:** [AssocThenDissoc::eval](../../../../crates/fit/src/library/pharma.rs#L1456-L1478) (which has no `default_bounds()`) evaluates `(-kobs * t0).exp()` at `#L1471` without checking `> 700.0`, and [DissociationOnePhase](../../../../crates/fit/src/library/pharma.rs#L1347-L1355), [AssociationOneConc](../../../../crates/fit/src/library/pharma.rs#L1383-L1391), [AssociationTwoConc](../../../../crates/fit/src/library/pharma.rs#L1420-L1429), and `AssocThenDissoc` guard only `arg > 700.0` (false on `NaN`) without final `.is_finite()` checks. Dynamic `probe_06_assoc_then_dissoc_and_kinetics_nonfinite` verified `assoc_then_dissoc` at `[bmax=100, kon=1e6, koff=0.1, conc=1.0, t0=-1e-3]` returns `Some(f64::NEG_INFINITY)` at `t = 0.0` and `Some(f64::NAN)` at `t = 1e4`, and all four kinetic models return `Some(f64::NAN)` when given `f64::NAN`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` (non-finite `eval` return values on unconstrained negative `t0` steps or `NaN` direct arguments).
- **Still present on main:** yes

### Verdict `FA-SEC08-07` · `CONFIRMED`

- **Verifier:** `ver-backend-3`
- **Evidence:** [strip_comment](../../../../crates/fit/src/dsl/statements.rs#L27-L55) iterates `for marker in ['#', ';']` and returns at the first match, slicing at `#` even when `;` occurs earlier on the line. Dynamic `probe_07_strip_comment_hash_before_semicolon` confirmed `Y = A * X ; sample #1` fails compilation with `FitError::DslCompileError` (`Unexpected character ';'`) whereas `Y = A * X # sample ;1` compiles `Ok`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` (unexpected syntax rejection on `;` comments containing `#`; workaround is using `#` comments).
- **Still present on main:** yes

### Verdict `FA-SEC08-08` · `CONFIRMED`

- **Verifier:** `ver-backend-3`
- **Evidence:** [resolve_equation_branches](../../../../crates/fit/src/dsl/statements.rs#L942-L949) only errors when `ode_init_branches.is_empty()`, and [CompiledDslModel::compile](../../../../crates/fit/src/dsl/model.rs#L194-L197) (`#L233-L273`) synthesizes a default `Y0` only when `ode_init_branches.is_empty()`. Thus when `<A>` defines an IC while `<B>` has an ODE derivative without an IC, `default_ode_init_idx` stays `None` and dataset `<B>` fails at runtime in [select_ode_init](../../../../crates/fit/src/dsl/model.rs#L456) without a compile error. Dynamic `probe_08_multi_branch_ode_omits_missing_ic_validation` verified `<A> Y0 = yA \n <A> dY/dX = -k1 * Y \n <B> dY/dX = -k2 * Y` compiles `Ok` without `Y0` in `param_names()`, returns `Some` for `ds=0`, and returns `None` for `ds=1`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` (latent compile-time omission causing silent runtime `eval` `None` failures on IC-less branches; workaround is supplying per-branch `Y0` / `Y(t0)`).
- **Still present on main:** yes

### Verdict `FA-SEC08-09` · `CONFIRMED`

- **Verifier:** `ver-backend-3`
- **Evidence:** [WeibullPeak::eval](../../../../crates/fit/src/library/curves.rs#L980-L992) unconditionally returns `Some(0.0)` when `x0 == 0.0` (even when `Shape == 1.0`, whose limit at `0+` is `Amplitude / Scale`, or `0 < Shape < 1`, which diverges at `0+`), and returns `Some(...)` on line `991` without checking `.is_finite()`. Dynamic `probe_09_weibull_peak_discontinuity_at_zero_and_inf_overflow` verified `[Amp=100, Scale=2, Shape=1]` returns `Some(0.0)` at `X=0.0` vs `Some(50.0)` at `X=1e-12`, and `[Amp=1e100, Scale=1, Shape=0.1]` returns `Some(f64::INFINITY)` at `X=1e-300`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` (`X=0` discontinuity at `Shape <= 1` and unguarded `+Inf` overflow for tiny positive `X` with `Shape < 1`).
- **Still present on main:** yes

### Verdict `FA-SEC08-10` · `CONFIRMED`

- **Verifier:** `ver-backend-3`
- **Evidence:** All four helpers omit finite-point filtering compared to `data_y_min_max` / `compute_dataset_stats`: [estimate_x_at_ymid](../../../../crates/fit/src/library/mod.rs#L158-L179), [GaussianCurve::initial_guess](../../../../crates/fit/src/library/curves.rs#L809), [estimate_global_ns_and_bg](../../../../crates/fit/src/library/pharma.rs#L1766-L1785), and [CompiledDslModel::initial_guess](../../../../crates/fit/src/dsl/model.rs#L585). Dynamic `probe_10_initial_guess_unfiltered_nonfinite_points` confirmed `NaN` seeds from `log_agonist_vs_response_4pl` on `[xy(1.0, 10.0), xy(NaN, 30.0)]`, corrupted `Mean = 999.0` on `gaussian` with `(999.0, NaN)`, `NaN` background on `one_site_total_and_ns_global`, and `0.1` (from `f64::NAN.max(0.1)`) / `f64::INFINITY` on `CompiledDslModel`. (*Precision note:* `estimate_x_at_ymid` short-circuits if an earlier finite window spans `y_mid`; also, `extract_datasets_from_sheet` strips non-finite sheet cells while direct `fit_nonlinear` callers enter `ParameterLayout::build` at [layout.rs](../../../../crates/fit/src/solver/layout.rs#L65-L70) prior to `build_observations`, where `clamp_to_bounds` at `#L311` converts non-finite `init` to `0.0` clamped to bounds.)
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S4` (minor public-trait contract asymmetry on direct `initial_guess` calls with non-finite `DataPoint`s).
- **Still present on main:** yes

### Verdict `FA-SEC08-11` · `CONFIRMED`

- **Verifier:** `ver-backend-3`
- **Evidence:** [is_builtin_constant_or_var](../../../../crates/fit/src/dsl/statements.rs#L1154-L1160) treats any `X<digits>` other than `"0"` as a reserved predictor, so `collect_free_idents` omits `X00` and overflowing `X999...9` from `param_map`, whereas [compile_ident](../../../../crates/fit/src/dsl/compile.rs#L258-L266) requires `rest.parse::<usize>()` and `idx >= 1`. Dynamic `probe_11_x00_predictor_divergence_and_opcode_budget_msg` confirmed both `"Y = X00 * a"` and `"Y = X999999999999999999999999999 * a"` fail codegen with `FitError::DslCompileError` (`Unknown symbol ...`).
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S4` (diagnostic/contract mismatch on degenerate `X00` / overflowing predictor identifiers).
- **Still present on main:** yes

### Verdict `FA-SEC08-12` · `CONFIRMED`

- **Verifier:** `ver-backend-3`
- **Evidence:** [CompiledDslModel::eval_implicit](../../../../crates/fit/src/dsl/model.rs#L321-L325) documents `"accept only |F| < 1e-7"` while [implicit_newton_fallback](../../../../crates/fit/src/dsl/implicit.rs#L69, #L107) and `verify_implicit_root` ([model.rs](../../../../crates/fit/src/dsl/model.rs#L374-L375)) use scale-invariant relative tolerances; separately [check_opcode_budget](../../../../crates/fit/src/dsl/compile.rs#L14-L23) hardcodes `{MAX_TOTAL_OPCODES}` (`100000 opcodes`) even when called with `remaining < MAX_TOTAL_OPCODES`. Dynamic `probe_11_x00_predictor_divergence_and_opcode_budget_msg` verified a two-branch `<A>`/`<B>` DSL exceeding cumulative opcodes across branches reports `"100000 opcodes"`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S4` (stale docstring and hardcoded budget limit string in error text).
- **Still present on main:** yes
