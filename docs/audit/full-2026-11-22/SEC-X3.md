# SEC-X3 Audit Report — Integration Test Suite & Policy Gate Integrity

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Scope:** `SEC-X3` — Cross-cutting audit of workspace integration tests (`308` `.rs` test files across `crates/*/tests/`, `apps/*/tests/`, and `xtask/tests/`, plus `9` `.py`/`.R` oracle reference scripts = `317` files) and all `15` `cargo xtask` policy checks (`CARGO_TARGET_DIR=/tmp/cb-audit-x3`)
- **Date:** `2026-11-22`
- **Mode:** Report-only (`SEC-X3` deliverable only; source/test/config files untouched)
- **Summary Counts:** `S1 = 0`, `S2 = 6`, `S3 = 3`, `S4 = 1` (**Total:** `10` findings: `FA-X3-01` .. `FA-X3-10`)

---

## 1. Policy Gate Baseline Results (`cargo xtask`)

Executed inside the repository root with `CARGO_TARGET_DIR=/tmp/cb-audit-x3` on `7c85a8e516165ddadb101236a88eec937bf1f095`. All `14/15` checks that target tracked `main` files exited `0`. Only `cargo xtask secrets` exited `1`, with **0 hits on tracked `main` files** and `443` hits solely on local macOS home-path markdown links inside untracked sibling audit files under `docs/audit/full-2026-11-22/` (details in `FA-X3-06`).

| # | Command | Exit | Time (s) | Observed Summary & Main vs Untracked Distinction |
|---|---|---:|---:|---|
| 1 | `cargo xtask layers` | `0` | `5.37` | `layers: all 22 workspace packages satisfy L0..L7 dependency rules and source hygiene` |
| 2 | `cargo xtask assets` | `0` | `1.77` | `assets: all 51 asset files attributed in ASSETS.md with clean provenance and allowlisted licences` |
| 3 | `cargo xtask brands` | `0` | `23.19` | `brands: 0 unapproved brand or suite mentions across 1089 tracked files and 0 dist/ files (0 archive entries) (13872 i18n entries, 16-locale live egui shell, MCP headless+gui-only JSON-RPC surfaces, 5 sheets, 26 dialogs, 7 inspectors, and export/icon metadata clean)` *(note: skips `docs/audit/`)* |
| 4 | `cargo xtask cleanroom` | `0` | `19.66` | `cleanroom: no vendor copyright notices or vendor-created fixtures found across 1089 tracked files and 0 dist/ files (0 archive entries)` *(note: skips `docs/audit/`)* |
| 5 | `cargo xtask cursors` | `0` | `1.31` | `cursors: all 5 Cursor variants verified 1-to-1 with crates/tools/src/cursors.toml` |
| 6 | `cargo xtask ui-commands --check` | `0` | `1.26` | `ui-commands: crates/mcp/src/ui_catalog.rs is in sync with crates/ui-egui/src/menus/ui_commands.rs` |
| 7 | `cargo xtask secrets` | `1` | `2.71` | `xtask error: 443 potential secret or local home-path exposure(s) found` — **0 hits in tracked `main`**; all `443` hits are local home-path URIs inside untracked `docs/audit/full-2026-11-22/SEC-*.md` files (see `FA-X3-06`) |
| 8 | `cargo xtask licenses` | `0` | `4.40` | `licenses ok` |
| 9 | `cargo xtask telemetry` | `0` | `2.29` | `telemetry: 0 violations across checks 1-5 (24 manifests, 431 locked packages, 389 Rust sources, 52 web/workflow/packaging files, 7 workflows, 2 header configs)` |
| 10 | `cargo xtask workflows` | `0` | `2.50` | `workflows: actionlint (7 workflow files) and shellcheck (12 shell scripts) passed` |
| 11 | `cargo xtask commands --check` | `0` | `1.04` | `commands: docs/commands.md is up to date` |
| 12 | `cargo xtask schemas --check` | `0` | `1.26` | `schemas: 5 schema file(s) in docs/schemas are up to date` |
| 13 | `cargo xtask status` | `0` | `1.48` | `status: verified 12 Markdown table(s) across docs/STATUS.md and docs/ROADMAP.md, live metrics, [x] evidence in docs/evidence/user-interface.md, and §6.2 Human path column in docs/gui-workflows.md (65/65 egui; backed human tests: 65/65 egui; five-stage complete: 65/65)` |
| 14 | `cargo xtask pending` | `0` | `1.06` | `pending: 88 open sub-task(s) across 33 roadmap item(s)` |
| 15 | `cargo xtask screenshots --check` | `0` | `1.31` | `workspace-split-fit.png OK`, `workspace-layout-dark.png OK`, `curvebook.png OK` |

---

## 2. Test Suite Coverage Table

Full AST/regex scan across all `22` workspace packages (`20` non-empty `tests/` directories totaling `308` `.rs` files + `9` `.py`/`.R` oracle reference scripts = `317` files; `crates/platform` and `crates/testkit` contain `0` integration tests), paired with targeted manual deep reads on high-risk suites. Cross-cutting `SEC-X3` avoids duplicating single-crate production defects already recorded under `FA-SEC01`..`FA-SEC23` in `docs/audit/full-code-audit-ledger-2026-11-22.md`.

| Package / Directory | `.rs` Files | Ref Scripts | Deep-Read Suites | Automated Scans Run | Findings / Status |
|---|---:|---:|---|---|---|
| `crates/chart` | 14 | 0 | `audit_r2_s08_s09.rs` | Empty-assert, discarded fallibles, skips, `#[ignore]`, D1 edges | Clean (test harness sound) |
| `crates/doc` | 9 | 0 | `audit_r2_s05_s07.rs` | Same | Clean |
| `crates/engine` | 35 | 0 | `registry_and_errors.rs`, `audit_r2_s05_s07.rs` | Same + `is_dirty` coverage check | `FA-X3-09` (missing `is_dirty` checks on open/save) |
| `crates/fit` | 27 | 3 `.py` | `d1_edge_matrix.rs`, `d1_fit_uncertainty.rs`, `d1_non_nist_fitting.rs`, `dsl_fuzz_props.rs`, `audit_r2_s03.rs`, `reference/*.py` | Same + D1 ties/`NaN`/`±Inf`/weights scan | `FA-X3-09` (`0/27` tests cover replicate/predictor ties) |
| `crates/format` | 16 | 0 | `audit_wave1_format_io.rs` | Same | Clean |
| `crates/math` | 9 | 1 `.py` | `audit_foundation_math.rs`, `d1_distribution_tails.rs`, `reference/distribution_tails.py` | Same | Clean |
| `crates/mcp` | 9 | 0 | `gui_only_surface.rs`, `audit_r2_s12.rs` | Same + TCP `PermissionDenied` skips & mock host checks | `FA-X3-03`, `FA-X3-04` |
| `crates/palette` | 1 | 0 | `audit_r1_s4b_palette.rs` | Same | Clean |
| `crates/pdf` | 5 | 0 | `audit_r1_s4b_s09.rs` | Same + `external_viewer_qa.rs` unguarded skip scan | `FA-X3-03` |
| `crates/platform` | 0 | 0 | *(no `tests/` directory)* | N/A | Clean (N/A) |
| `crates/render` | 5 | 0 | `audit_r2_s08_s09.rs` | Same | Clean |
| `crates/stats` | 33 | 1 `.R` | `d1_edge_matrix.rs`, `d1_poisson_cox_result_sheets.rs`, `audit_r1_s4b_stats.rs`, `audit_r2_s02.rs`, `audit_twogroup_col_fixes.rs`, `reference/poisson_cox_result_sheet.R` | Same + Pratt/bugs-asserted check | `FA-X3-09` (`use_pratt_zeros = false` asserted) |
| `crates/svg` | 3 | 0 | `audit_r1_s4b_s09.rs` | Same | Clean |
| `crates/testkit` | 0 | 0 | *(no `tests/` directory)* | N/A | Clean (N/A) |
| `crates/text` | 6 | 0 | `without_bundled_fonts.rs` | Same | Clean |
| `crates/tools` | 4 | 0 | `audit_r1_s4b_tools.rs`, `audit_r2_s08_s09.rs` | Same | Clean |
| `crates/transform` | 14 | 4 `.py`/`.R` | `d1_curve_transforms.rs`, `d1_randomization.rs`, `audit_r2_s04.rs`, `reference/*` | Same + D1 `NaN`/`±Inf` scan | `FA-X3-09` (`0/14` `.rs` files test `±Inf`) |
| `crates/ui-egui` | 76 | 0 | `r15_4_gui_only_wf_egui.rs`, `r15_2_anova_and_1m_bench.rs`, `audit_r2_s10.rs`, `r10_core_and_spike.rs`, `human_wf_common/mod.rs`, `human_wf_{01_06,07_12,19_24,25_31,32_37,38_43,44_50,51_58,59_65}.rs`, `gui_wf_{statistical_graphs,regression_contingency_multivariate}.rs`, `r10_human_wf_scaffold_smoke.rs`, `rv_ledger_third_pass_ui.rs`, `r10_23_beta_desktop_gaps.rs`, `r15_6_wave_b_grid_vars.rs`, `r15_6_phase1_pipeline.rs`, `cjk_rtl_hidpi_desktop_qa.rs` | Same + `let _ =` command/locale, `#[ignore]`, `if let Some` assert skips | `FA-X3-01`, `FA-X3-02`, `FA-X3-03`, `FA-X3-04`, `FA-X3-05`, `FA-X3-07`, `FA-X3-10` |
| `apps/curvebook` | 12 | 0 | `control_loopback.rs`, `recovery_wiring.rs`, `r73_shared_window.rs`, `audit_r2_desktop_host.rs` | Same + loopback `TimedOut`/`PermissionDenied` scan | `FA-X3-03` |
| `apps/curvebook-cli` | 18 | 0 | `audit_r2_s12.rs`, `mcp_gui_only_parity.rs`, `r73_mcp_surface_and_headless.rs`, `r73_mcp_gui_only_wf_{01_15,16_28,29_42,43_50}.rs` | Same + MCP PCR/graph workarounds & `PermissionDenied` skips | `FA-X3-02`, `FA-X3-03`, `FA-X3-04` |
| `apps/curvebook-web` | 5 | 0 | `browser_host_adapters.rs`, `human_wf_web_parity.rs`, `r10_7_web_shared_layout.rs`, `rv53_filename_sanitize.rs`, `web_parity_common/mod.rs` | Same + web parity / UI assertion check | `FA-X3-07`, `FA-X3-08` |
| `xtask` | 7 | 0 | `audit_r2_s13.rs`, `r014_release_channel.rs`, `r021_strict_alpha_branding.rs`, `r026_release_scan.rs`, `r028_beta_assets.rs`, `r032_decoupled_gates.rs`, `r12_docs_site.rs` | `xtask/src` & `tests` audit (`ci.rs`, `secrets.rs`, `brands.rs`, `cleanroom.rs`, `main.rs`) | `FA-X3-05`, `FA-X3-06` |
| **Total** | **308** | **9** | — | — | **10 Findings (`FA-X3-01`..`FA-X3-10`)** |

---

## 3. Confirmed Findings (`S1` -> `S4`)

### `FA-X3-01` (`S2`) — Hollow `WF-30` formula-bar test (`let _ =` swallowing `Err(Conflict)`) & 25 assertion-minimal `verify_egui_inspect_and_shot` tests

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/ui-egui/tests/r15_4_gui_only_wf_egui.rs:228-239, 782-790` (with production dispatch in `crates/ui-egui/src/control_io.rs:3040-3098` and `crates/ui-egui/src/gui_actions.rs:2154-2156`)
- **Category:** Discarded fallible action / existence-only assertions in GUI-only workflow suite
- **Defect:** `wf30_egui_gui_only_calculated_variables_formula_bar` discards the result of `c.gui("ui.answer_dialog", ...)` via `let _ =`. In production, `dispatch_ui_open_formula_bar` (`control_io.rs:3040-3098`) sets `app.grid.formula_bar.active = true` rather than opening `canvas_dialogs.active_dialog`, so `handle_answer_dialog` (`gui_actions.rs:2154-2156`) always returns `Err(Conflict, "no modal dialog is currently open")` — which `wf30` ignores. Additionally, `verify_egui_inspect_and_shot` (`228-239`) only validates `gui_only == Some(true)`, `active_sheet_id == Some(expected_sheet)`, `menus.len() == 8`, and PNG screenshots; `25` tests in this suite (`wf16`, `wf18`, `wf20`, `wf21`, `wf22`, `wf23`, `wf24`, `wf25` [which also skips SVG export at `702-713`], `wf26`, `wf27`, `wf29`, `wf30`, `wf31`, `wf32`, `wf33`, `wf34`, `wf35`, `wf38`, `wf43`, `wf44`, `wf45`, `wf46`, `wf47`, `wf48`, `wf49`) end with `verify_egui_inspect_and_shot` without inspecting `ResultSheet` values, scalars, or `GraphSpec` outputs.
- **Failure scenario:** The formula bar commit fails completely at runtime, or an analysis/graph workflow leaves sheets blank or produces wrong numbers, yet `r15_4_gui_only_wf_egui` reports full green because the dialog error is discarded and only menu count + active sheet name + screenshot capture are asserted.
- **Evidence:**
```rust
// crates/ui-egui/tests/r15_4_gui_only_wf_egui.rs:228-239, 782-790
fn verify_egui_inspect_and_shot(c: &mut EguiGuiClient, expected_sheet: &str) -> Result<Vec<u8>, String> {
    let snap = c.inspect("all")?;
    ensure(snap.get("gui_only").and_then(Value::as_bool) == Some(true), "expected gui_only=true in ui.inspect")?;
    ensure(
        snap.get("active_sheet_id").and_then(Value::as_str) == Some(expected_sheet),
        format!("expected active_sheet_id `{expected_sheet}`, got {:?}", snap.get("active_sheet_id")),
    )?;
    ensure(snap.get("menus").and_then(Value::as_array).is_some_and(|m| m.len() == 8), "expected 8 top-level menus in ui.inspect")?;
    let png = c.screenshot_png("full_window")?;
    let _canvas_png = c.screenshot_png("canvas")?;
    Ok(png)
}
...
#[test]
fn wf30_egui_gui_only_calculated_variables_formula_bar() -> TestRes {
    let mut c = EguiGuiClient::new()?;
    let sid = c.create_table(json!({ "sheet_id": "mv-30", "kind": "multiple_variables", "cols": 3, "rows": 4 }))?;
    c.paste_tsv(0, 0, 0, "10\t2\t0\n20\t4\t0\n30\t6\t0\n40\t8\t0")?;
    c.menu_cmd("ui.table.openFormulaBar", json!({ "column": 2 }))?;
    let _ = c.gui("ui.answer_dialog", json!({ "action": "ok", "fields": { "formula": "[Col1] / [Col2]" } }));
    verify_egui_inspect_and_shot(&mut c, &sid)?;
    Ok(())
}
```
- **Suggested fix:** Remove `let _ =` in `wf30` and drive/assert actual formula-bar entry submission (verifying computed column cell values `[5, 5, 5, 5]`), and upgrade the 25 assertion-minimal tests to validate post-workflow sheet tabs, `ResultSheet` scalars/rows, or `GraphSpec` properties rather than shell metadata alone.
- **Related:** `crates/ui-egui/src/control_io.rs:3040-3098`, `crates/ui-egui/src/gui_actions.rs:2154-2156`.

---

### `FA-X3-02` (`S2`) — Systemic `WF-23` PCR & `ExportFigure` modal test blind spots masking open UI defects

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/ui-egui/tests/human_wf_19_24.rs:843-845, 906-912`; `crates/ui-egui/tests/human_wf_25_31.rs:522-525`; `crates/ui-egui/tests/gui_wf_regression_contingency_multivariate.rs:1158-1168`; `crates/ui-egui/tests/r15_4_gui_only_wf_egui.rs:682-684`; `apps/curvebook-cli/tests/r73_mcp_gui_only_wf_16_28.rs:266-267, 272, 350-353`
- **Category:** Missing output/outcome assertions in critical-path workflow tests
- **Defect:** Across four independent suites, `WF-23` (Principal Components Regression) and `WF-24`/`WF-28` (`ExportFigure` SVG export) tests do not inspect the produced output:
  - `human_wf_19_24.rs:843-845` runs `run.click("analyze.pcr.outcome.2")?; run.commit_modal_ok()?;` without checking any `ResultSheet` output.
  - `r73_mcp_gui_only_wf_16_28.rs:266-267, 272, 350-353` invokes `entry_id: "stats.pca"`, patches `"pcr_outcome_column": 2`, and checks only `!tabs.is_empty()`.
  - `gui_wf_regression_contingency_multivariate.rs:1158-1168` invokes `entry_id: "stats.pca"`, sets `pcr_outcome_column: 2`, and checks only modal closure + screenshot.
  - `r15_4_gui_only_wf_egui.rs:682-684` (`wf23`) calls `run_analyze` followed by `verify_egui_inspect_and_shot`.
  - `human_wf_19_24.rs:906-912` (`wf24`) and `human_wf_25_31.rs:522-525` (`wf28`) open `ExportFigure`, choose SVG, call `commit_modal_ok()`, and assert only dialog dismissal (`wf24`) or nothing (`wf28`).
- **Failure scenario:** Broken PCR execution/routing or non-functional SVG export modal commit slips through CI undetected across all GUI and MCP workflow suites.
- **Evidence:** Direct inspection of `human_wf_19_24.rs:843-845, 906-912`, `human_wf_25_31.rs:522-525`, `r73_mcp_gui_only_wf_16_28.rs:266-272, 350-353`, `gui_wf_regression_contingency_multivariate.rs:1158-1168`, and `r15_4_gui_only_wf_egui.rs:682-684`.
- **Suggested fix:** Assert the created PCR `ResultSheet` coefficients/$R^2$/outcome binding in all four `WF-23` tests, and verify non-empty exported SVG file bytes/DOM on disk after `ExportFigure` `commit_modal_ok()` in `wf24` and `wf28`.
- **Related:** `FA-SEC14-03`, `FA-SEC18-01`, `FA-SEC19-02` (`docs/audit/full-code-audit-ledger-2026-11-22.md`).

---

### `FA-X3-03` (`S2`) — Loopback TCP suites treat `TimedOut`/`PermissionDenied` as `Ok(())`, and external-viewer / macOS-clipboard tests skip unconditionally without CI guards

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:**
  - `apps/curvebook/tests/control_loopback.rs:263-264, 365-366, 432-433, 580-581, 674-675, 740-741`
  - `apps/curvebook/tests/recovery_wiring.rs:236-237`
  - `apps/curvebook/tests/r73_shared_window.rs:72, 192, 209, 256, 295, 510, 623, 703`
  - `apps/curvebook-cli/tests/{audit_r2_s12.rs:229,234; mcp_gui_only_parity.rs:190,196,416; r73_mcp_surface_and_headless.rs:598,604,656}`
  - `crates/mcp/tests/{audit_r2_s12.rs:227,234; gui_only_surface.rs:362,377; mcp_e2e.rs:607,624}`
  - `crates/pdf/tests/external_viewer_qa.rs:80-90, 297-304, 446-452, 521-529`
  - `crates/ui-egui/tests/macos_clipboard_live.rs:37-82`
  - `.github/workflows/ci.yml:1-211`
- **Category:** Silent test skips / unguarded timeout suppression
- **Defect:** Desktop loopback tests (`control_loopback.rs`, `recovery_wiring.rs`, `r73_shared_window.rs`) return `Ok(())` whenever socket bind/connect/RPC yields `ErrorKind::PermissionDenied` **or `ErrorKind::TimedOut`**, without checking `CI` or a `CURVEBOOK_REQUIRE_*` fail-closed env var. MCP suites (`crates/mcp/tests/`, `apps/curvebook-cli/tests/`) similarly return `Ok(())` unguarded on `PermissionDenied`. Likewise, `crates/pdf/tests/external_viewer_qa.rs` (`pdftoppm`/`gs`) and `crates/ui-egui/tests/macos_clipboard_live.rs` execute unconditional `SKIP` returns when host tools/display are absent, and `.github/workflows/ci.yml` never installs `pdftoppm`/`gs`.
- **Failure scenario:** A hang or regression in the desktop control loopback protocol causes `ErrorKind::TimedOut`, or CI lacks `pdftoppm`/`gs`, yet all affected integration suites quietly pass with `Ok(())`.
- **Evidence:** Direct inspection of the listed lines across `apps/curvebook/tests/`, `apps/curvebook-cli/tests/`, `crates/mcp/tests/`, `crates/pdf/tests/external_viewer_qa.rs`, `crates/ui-egui/tests/macos_clipboard_live.rs`, and `.github/workflows/ci.yml:1-211`.
- **Suggested fix:** Remove `ErrorKind::TimedOut => return Ok(())` from loopback tests (timeouts must fail loudly), gate any host-sandbox `PermissionDenied` or optional binary skips behind an explicit opt-out rather than silent `Ok(())` when `CI` or `CURVEBOOK_REQUIRE_*` is set, and install `pdftoppm` (`poppler-utils`) in `.github/workflows/ci.yml` so `external_viewer_qa` executes in CI.
- **Related:** `.config/nextest.toml:1-19`.

---

### `FA-X3-04` (`S2`) — Mock-only `mcp` test cements `width_px`/`height_px` schema drift, `assert_contract` omits post-patch readback, and `gui_wf` tests bypass broken `ChangeGraphType`

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/mcp/tests/gui_only_surface.rs:553, 588`; `crates/ui-egui/tests/audit_r2_s10.rs:182-195, 210`; `crates/ui-egui/tests/gui_wf_statistical_graphs.rs:344-346`; `apps/curvebook-cli/tests/r73_mcp_gui_only_wf_16_28.rs:304-313, 321-333`
- **Category:** Mock-host schema mismatch, incomplete contract readback, and test workaround bypassing broken UI action
- **Defect:**
  1. `crates/mcp/tests/gui_only_surface.rs:553, 588` asserts `"metrics": { "width_px": 1280.0, "height_px": 800.0, "scale_factor": 1.0 }` against `SharedMockHost` rather than production `ViewportMetrics` (`logical_width`, `logical_height`, `scale_factor`).
  2. `crates/ui-egui/tests/audit_r2_s10.rs:182-195, 210` patches `"duplicate.choice": "modify_existing"` and immediately calls `close_dialog()` without re-inspecting the control value (while production `inspect.rs:2343-2346` hardcodes `"create_new"`).
  3. `crates/ui-egui/tests/gui_wf_statistical_graphs.rs:344-346` and `apps/curvebook-cli/tests/r73_mcp_gui_only_wf_16_28.rs:304-313, 321-333` avoid `ChangeGraphType` / `run_graph_set_kind` (which discards `params.option_id`) by calling `graph.setSeries` directly.
- **Failure scenario:** Three distinct GUI/MCP defects remain green in tests: MCP clients receive mismatched viewport metric keys from real vs mock hosts, `duplicate.choice` silently stays `"create_new"`, and `ChangeGraphType` menu actions remain non-functional.
- **Evidence:** Direct inspection of `crates/mcp/tests/gui_only_surface.rs:553, 588`, `crates/ui-egui/tests/audit_r2_s10.rs:182-210`, `crates/ui-egui/tests/gui_wf_statistical_graphs.rs:344-346`, and `apps/curvebook-cli/tests/r73_mcp_gui_only_wf_16_28.rs:304-333`.
- **Suggested fix:** Align `SharedMockHost` metrics in `gui_only_surface.rs` with `ViewportMetrics` (`logical_width`/`logical_height`/`scale_factor`) and share one schema test against the real host; add post-patch `ui.inspect` readback assertions in `audit_r2_s10.rs`; test `ChangeGraphType` directly instead of bypassing with `graph.setSeries`.
- **Related:** `FA-SEC21-01`, `FA-SEC13-13` (`docs/audit/full-code-audit-ledger-2026-11-22.md`).

---

### `FA-X3-05` (`S2`) — Permanently `#[ignore]`d 1M-cell release Go/No-Go gate & `PERFORMANCE_FILTER` exclusion of `r15_2_anova_and_1m_bench`

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/ui-egui/tests/r15_2_anova_and_1m_bench.rs:284-315, 529-546`; `xtask/src/ci.rs:16`; `.github/workflows/ci.yml:1-211`
- **Category:** Ignored release gate & CI test-filter omission
- **Defect:** The 1M-cell release Go/No-Go benchmark assertions in `crates/ui-egui/tests/r15_2_anova_and_1m_bench.rs:284-315, 529-546` are gated by `#[ignore]`, and neither `.github/workflows/ci.yml` nor `xtask/src/ci.rs` invokes `--ignored` benchmark runs (comments in the test file explicitly document unresolved `views/grid_core.rs:323` clone latency ~38–55ms). Furthermore, `PERFORMANCE_FILTER` in `xtask/src/ci.rs:16` (`"binary(r10_*) | binary(human_wf_*) | binary(*perf*)"`) fails to match `r15_2_anova_and_1m_bench`, causing `r15_2_egui_1m_column_ci_smoke` to run in the noisy parallel `tests` CI stage rather than the serialized performance stage.
- **Failure scenario:** Release-blocking 1M-cell frame-time regressions stay untested in CI while the CI smoke test is exposed to parallel-execution CPU contention.
- **Evidence:** Direct inspection of `r15_2_anova_and_1m_bench.rs:284-315, 529-546`, `xtask/src/ci.rs:16`, and `.github/workflows/ci.yml:1-211`.
- **Suggested fix:** Add `| binary(r15_2_anova_and_1m_bench)` to `PERFORMANCE_FILTER` in `xtask/src/ci.rs:16`, and run the 1M-cell release gate explicitly in release/nightly CI instead of leaving it permanently `#[ignore]`d.
- **Related:** `xtask/src/ci.rs:1-223`.

---

### `FA-X3-06` (`S2`) — `cargo xtask secrets` fails (`EXIT=1`) on untracked audit docs; policy scope asymmetry across `repo_files` and `full-gates`

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `xtask/src/main.rs:172-190`; `xtask/src/secrets.rs:83`; `xtask/src/brands.rs:349`; `xtask/src/cleanroom.rs:195`; `xtask/src/ci.rs:155-167`
- **Category:** Policy gate baseline & scope asymmetry
- **Defect:** `repo_files()` (`xtask/src/main.rs:172-190`) returns untracked non-ignored files alongside tracked files, so `cargo xtask secrets` (`secrets.rs:83`) exits `1` (`443 potential secret or local home-path exposure(s) found`) whenever untracked `docs/audit/full-2026-11-22/SEC-*.md` files contain local macOS home-path markdown links (`0` violations exist on tracked `main`). Meanwhile, `brands.rs:349` and `cleanroom.rs:195` skip `docs/audit/` (so `brands`/`cleanroom` cannot catch accidental brand or cleanroom violations inside audit docs), `ci policy` (`xtask/src/ci.rs:155-167`) conditionally gates `commands`/`docs-site` behind `full-gates`, and `xtask/tests/r021_strict_alpha_branding.rs` / `r12_docs_site.rs` depend on the default `full-gates` feature.
- **Failure scenario:** Adding local audit markdown containing absolute workspace links breaks `cargo xtask secrets` locally (`EXIT=1`), while `brands`/`cleanroom` skip `docs/audit/` completely and provide zero protection for documentation artifacts written there.
- **Evidence:** Observed execution of all `15` `cargo xtask` checks (`14` `EXIT=0`; `secrets` `EXIT=1` with `443` hits strictly inside untracked `docs/audit/full-2026-11-22/SEC-*.md` and `0` hits on tracked `main`) plus inspection of `xtask/src/{main.rs,secrets.rs,brands.rs,cleanroom.rs,ci.rs}`.
- **Suggested fix:** Require relative repo paths in `docs/audit/full-2026-11-22/*.md` (remediating the `443` links in existing audit deliverables so `cargo xtask secrets` returns `EXIT=0`), and consider running `brands`/`cleanroom` with a docs-appropriate allowlist over `docs/audit/` rather than skipping the directory outright.
- **Related:** `FA-X3` Policy Gate Baseline table (§1).

---

### `FA-X3-07` (`S3`) — Unchecked `if let Some(...)` assertion skips and discarded `execute_command` in `ui-egui` & `curvebook-web` tests

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/ui-egui/tests/audit_r2_s10.rs:525-530, 535-540, 547-553, 855-857`; `crates/ui-egui/tests/r10_core_and_spike.rs:227-230`; `crates/ui-egui/tests/r15_2_anova_and_1m_bench.rs:483-492`; `apps/curvebook-web/tests/rv53_filename_sanitize.rs:57-61`
- **Category:** Vacuous conditional assertions (`if let Some`) & discarded command results
- **Defect:** Regression tests place assertions inside `if let Some(...)` without an `else { panic!(...) }` or `.expect(...)` check, and discard `execute_command` results, allowing the assertions to skip silently when the searched control/value/tree node is absent.
- **Failure scenario:** If a regression removes or breaks the target UI state so the lookup returns `None`, the test exits the `if let` branch cleanly and passes.
- **Evidence:** Direct inspection of `audit_r2_s10.rs:525-553, 855-857`, `r10_core_and_spike.rs:227-230`, `r15_2_anova_and_1m_bench.rs:483-492`, and `rv53_filename_sanitize.rs:57-61`.
- **Suggested fix:** Replace optional `if let Some(...)` guards around test assertions with `.expect(...)` / `assert!(...is_some())`, and assert `Ok(...)` on `execute_command`.
- **Related:** —

---

### `FA-X3-08` (`S3`) — `apps/curvebook-web/tests/human_wf_web_parity.rs` mislabels direct `ExecuteCommand` smoke tests as `web_human_*` `WF-51..65` parity

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `apps/curvebook-web/tests/human_wf_web_parity.rs:209-411`
- **Category:** Mislabeled UI/parity test coverage
- **Defect:** Tests named `web_human_*` (`WF-51..65`) invoke `exec_web` (`ExecuteCommand` directly on engine) with zero `UiInputEvent` dispatch, zero modal checks, and zero `ResultSheet`/scalar/status assertions — verifying only ZIP/SVG magic headers. `WF-01..49` have no coverage in `apps/curvebook-web/tests/`.
- **Failure scenario:** Web UI interactions, modal dialogs, or numerical analysis outputs on the web target regress completely while `human_wf_web_parity` continues to pass by exercising headless engine execution and container headers alone.
- **Evidence:** Direct inspection of `apps/curvebook-web/tests/human_wf_web_parity.rs:1-411`.
- **Suggested fix:** Rename or split direct-command container checks from true web UI parity tests, assert `ResultSheet`/scalar outputs on every analysis step, and route web UI parity through `UiInputEvent`/DOM surface checks alongside `WF-01..49` smoke parity.
- **Related:** `apps/curvebook-web/tests/web_parity_common/mod.rs:1-100`.

---

### `FA-X3-09` (`S3`) — D1 edge-case matrix gaps (`±Inf` absent in `transform`, ties absent in `fit`), buggy-behavior assertion (`use_pratt_zeros = false`), and missing `is_dirty()` checks

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/transform/tests/` (`0/14` `.rs` files cover `±Inf`); `crates/fit/tests/` (`0/27` `.rs` files cover predictor/replicate ties); `crates/stats/tests/audit_twogroup_col_fixes.rs:284-302` (`use_pratt_zeros = false` at line `301`); `crates/engine/tests/`
- **Category:** D1 numeric edge-matrix coverage gaps & outdated assertion of buggy behavior
- **Defect:** Systematic edge-case scan across core numerical crates shows:
  1. `crates/transform/tests/`: `0/14` `.rs` integration tests exercise `f64::INFINITY` / `f64::NEG_INFINITY` (`NaN` appears in `4/14` files), leaving non-finite transform regressions untested.
  2. `crates/fit/tests/`: `0/27` `.rs` tests exercise predictor/replicate ties, leaving ties-handling regressions untested.
  3. `crates/stats/tests/audit_twogroup_col_fixes.rs:284-302`: hardcodes `use_pratt_zeros = false` (`line 301`), asserting behavior contrary to documented Pratt zero handling.
  4. `crates/engine/tests/`: lacks `Session::is_dirty()` assertions around `project.open`/`save`.
- **Failure scenario:** Inf propagation bugs in transforms, singular/degenerate jackknife results on tied Deming inputs, incorrect Wilcoxon zero handling, and dirty-flag desync on open/save escape integration CI.
- **Evidence:** Workspace AST/regex edge-case scan across `crates/{fit,stats,transform,engine}/tests/` and inspection of `audit_twogroup_col_fixes.rs:275-325`.
- **Suggested fix:** Add `±Inf`/`NaN` cases to `crates/transform/tests/d1_curve_transforms.rs`, add tied-input cases (including `n = 3` Deming duplicate points) to `crates/fit/tests/d1_edge_matrix.rs`, update `audit_twogroup_col_fixes.rs` alongside fixing Pratt zero handling, and add `Session::is_dirty()` assertions on `project.open`/`save` in `crates/engine/tests/`.
- **Related:** `FA-SEC03-15`, `FA-SEC05-06`, `FA-SEC07-08`, `FA-SEC11-04` (`docs/audit/full-code-audit-ledger-2026-11-22.md`).

---

### `FA-X3-10` (`S4`) — Unchecked `let _ = app.set_locale(...)` across 15 UI test sites masks silent `"en"` fallback in `UiApp::set_locale`

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/ui-egui/src/lib.rs:549-555`; `crates/ui-egui/src/i18n.rs:204-233`; 15 test sites across `crates/ui-egui/tests/{human_wf_common/mod.rs:310,316; r15_2_anova_and_1m_bench.rs:594-595; cjk_rtl_hidpi_desktop_qa.rs:166,401; r10_23_beta_desktop_gaps.rs:291; r10_phase7_gates.rs:405,413; r15_4_r10_23_egui_gaps.rs:322,411,439,582; audit_r2_s10.rs:683}`
- **Category:** Silent locale fallback masked by discarded return value
- **Defect:** `UiApp::set_locale(&mut self, tag: &str) -> &str` silently falls back to `"en"` when a locale tag is unknown/unsupported (`i18n.rs:204-233`), and 15 test call sites discard the returned matched `&str` via `let _ = app.set_locale(...)`.
- **Failure scenario:** A typo or broken locale catalog normalization in a CJK/RTL/i18n test silently switches the app back to `"en"` while the test continues under the impression it is verifying the target locale.
- **Evidence:** Direct inspection of `crates/ui-egui/src/lib.rs:549-555`, `crates/ui-egui/src/i18n.rs:204-233`, and the 15 test call sites listed above.
- **Suggested fix:** Assert `assert_eq!(app.set_locale(expected), expected)` (or normalized tag) at every test call site instead of `let _ =`.
- **Related:** —
