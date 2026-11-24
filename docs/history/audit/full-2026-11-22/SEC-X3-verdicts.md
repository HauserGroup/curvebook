# Verification Report: SEC-X3 — Integration Test Suite & Policy Gate Integrity

- **Section:** `SEC-X3` (`308` `.rs` integration tests + `9` reference scripts across `22` workspace packages; `15` `cargo xtask` policy checks)
- **Source Report:** [`docs/audit/full-2026-11-22/SEC-X3.md`](SEC-X3.md)
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Verifier:** `ver-frontend-2`
- **Date:** `2026-11-22`

## Summary Table

| Verdict | Count | Settled Findings |
|---|---:|---|
| `CONFIRMED` | 10 | `FA-X3-01`, `FA-X3-02`, `FA-X3-03`, `FA-X3-04`, `FA-X3-05`, `FA-X3-06`, `FA-X3-07`, `FA-X3-08`, `FA-X3-09`, `FA-X3-10` |
| `PLAUSIBLE` | 0 | — |
| `REFUTED` | 0 | — |
| `DUPLICATE` | 0 | — |

| Settled Severity | Count | Findings |
|---|---:|---|
| `S1` | 0 | — |
| `S2` | 6 | `FA-X3-01`, `FA-X3-02`, `FA-X3-03`, `FA-X3-04`, `FA-X3-05`, `FA-X3-06` |
| `S3` | 3 | `FA-X3-07`, `FA-X3-08`, `FA-X3-09` |
| `S4` | 1 | `FA-X3-10` |

## Scope & Test Inventory Completeness Check

Verified 100% completeness of both [`SEC-X3.md`](SEC-X3.md#L11-L64) baseline tables (`PASS`):

1. **Test Inventory (`22` workspace packages, `308` `.rs` files + `9` `.py`/`.R` scripts = `317` files):** Independently counted across `crates/*/tests/`, `apps/*/tests/`, and `xtask/tests/`:
   - `crates/ui-egui` (`76`), `crates/engine` (`35`), `crates/stats` (`33` + `1` `.R`), `crates/fit` (`27` + `3` `.py`), `apps/curvebook-cli` (`18`), `crates/format` (`16`), `crates/chart` (`14`), `crates/transform` (`14` + `4` `.py`/`.R`), `apps/curvebook` (`12`), `crates/doc` (`9`), `crates/math` (`9` + `1` `.py`), `crates/mcp` (`9`), `xtask` (`7`), `crates/text` (`6`), `apps/curvebook-web` (`5`), `crates/pdf` (`5`), `crates/render` (`5`), `crates/tools` (`4`), `crates/svg` (`3`), `crates/palette` (`1`), `crates/platform` (`0`), `crates/testkit` (`0`).
2. **Independent `cargo xtask` Execution (`CARGO_TARGET_DIR=/tmp/cb-audit-vx3`, cleaned up after completion):**
   - `14 / 15` checks exited `0` (`layers`, `assets`, `cursors`, `ui-commands --check`, `licenses`, `telemetry`, `workflows`, `commands --check`, `schemas --check`, `status`, `pending`, `screenshots --check`, `brands`, `cleanroom`).
   - `cargo xtask secrets` exited `1` (`496` local home-path exposure hits in working tree including untracked audit markdown; `git grep -n '/Users/'` confirmed `0` real secret/home-path exposures on tracked `main` files, only existing allowlisted template placeholders).

---

## Per-Finding Verdicts

### Verdict `FA-X3-01` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` — `wf30` discards an unconditional `Err(Conflict)` from `ui.answer_dialog` on the formula bar, and 25 tests in `r15_4_gui_only_wf_egui` never inspect `ResultSheet` outputs or `GraphSpec` results.
- **Evidence:**
  - [`crates/ui-egui/tests/r15_4_gui_only_wf_egui.rs:782-790`](../../../../crates/ui-egui/tests/r15_4_gui_only_wf_egui.rs#L782-L790): `wf30` opens `ui.table.openFormulaBar` and discards `let _ = c.gui("ui.answer_dialog", ...)`. [`dispatch_ui_open_formula_bar`](../../../../crates/ui-egui/src/control_io.rs#L3040-L3098) activates `app.grid.formula_bar.active` without opening `canvas_dialogs.active_dialog`, so [`handle_answer_dialog`](../../../../crates/ui-egui/src/gui_actions.rs#L2154-L2156) unconditionally returns `Err(Conflict, "no modal dialog is currently open")`.
  - [`r15_4_gui_only_wf_egui.rs:228-239, 581-1136`](../../../../crates/ui-egui/tests/r15_4_gui_only_wf_egui.rs#L228-L1136): Confirmed all `25` enumerated tests (`wf16`, `wf18`, `wf20..27`, `wf29..35`, `wf38`, `wf43..49`) terminate in `verify_egui_inspect_and_shot` without checking any `ResultSheet` scalars/rows or `GraphSpec` properties (with `wf25` omitting SVG export and `wf29` checking only filtered `row_count() == 2`).

---

### Verdict `FA-X3-02` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` — `WF-23` PCR and `WF-24`/`WF-28` `ExportFigure` SVG workflow tests omit output assertions across four suites, masking `FA-SEC14-03`, `FA-SEC18-01`, and `FA-SEC19-02`.
- **Evidence:**
  - **PCR (`WF-23`):** [`crates/ui-egui/tests/human_wf_19_24.rs:843-849`](../../../../crates/ui-egui/tests/human_wf_19_24.rs#L843-L849), [`crates/ui-egui/tests/gui_wf_regression_contingency_multivariate.rs:1158-1168`](../../../../crates/ui-egui/tests/gui_wf_regression_contingency_multivariate.rs#L1158-L1168), [`apps/curvebook-cli/tests/r73_mcp_gui_only_wf_16_28.rs:266-277, 350-353`](../../../../apps/curvebook-cli/tests/r73_mcp_gui_only_wf_16_28.rs#L266-L353), and [`r15_4_gui_only_wf_egui.rs:678-685`](../../../../crates/ui-egui/tests/r15_4_gui_only_wf_egui.rs#L678-L685) never inspect PCR coefficients/$R^2$/outcome variables in the resulting `ResultSheet`.
  - **`ExportFigure` SVG (`WF-24`/`WF-28`):** [`human_wf_19_24.rs:906-912`](../../../../crates/ui-egui/tests/human_wf_19_24.rs#L906-L912) and [`human_wf_25_31.rs:522-525`](../../../../crates/ui-egui/tests/human_wf_25_31.rs#L522-L525) only assert dialog dismissal (`wf24`) or `commit_modal_ok()` success (`wf28`), never verifying SVG output bytes/files on disk.

---

### Verdict `FA-X3-03` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` — Desktop loopback tests treat `ErrorKind::TimedOut` and `PermissionDenied` as `Ok(())`, and external-viewer / macOS-clipboard tests skip unconditionally without CI fail-closed guards.
- **Evidence:**
  - Verified silent `ErrorKind::TimedOut | ErrorKind::PermissionDenied => return Ok(())` (or `Ok(None)`) across [`apps/curvebook/tests/control_loopback.rs:263-264, 365-366, 432-433, 580-581, 674-675, 740-741`](../../../../apps/curvebook/tests/control_loopback.rs#L263-L741), [`recovery_wiring.rs:236-237`](../../../../apps/curvebook/tests/recovery_wiring.rs#L236-L237), and [`r73_shared_window.rs:72, 192, 209`](../../../../apps/curvebook/tests/r73_shared_window.rs#L72-L209) (contradicting [`.config/nextest.toml:5`](../../../../.config/nextest.toml#L5) `"A timeout is a failing test"`).
  - Verified unguarded `PermissionDenied => return Ok(())` in [`apps/curvebook-cli/tests/audit_r2_s12.rs:229, 234`](../../../../apps/curvebook-cli/tests/audit_r2_s12.rs#L229-L236), [`mcp_gui_only_parity.rs:190, 196, 416`](../../../../apps/curvebook-cli/tests/mcp_gui_only_parity.rs#L190-L416), [`r73_mcp_surface_and_headless.rs:598, 604, 656`](../../../../apps/curvebook-cli/tests/r73_mcp_surface_and_headless.rs#L598-L656), and [`crates/mcp/tests/audit_r2_s12.rs:227, 234`](../../../../crates/mcp/tests/audit_r2_s12.rs#L227-L235) (**precision note:** [`crates/mcp/tests/gui_only_surface.rs:362, 377`](../../../../crates/mcp/tests/gui_only_surface.rs#L362-L385) and [`mcp_e2e.rs:607, 624`](../../../../crates/mcp/tests/mcp_e2e.rs#L607-L630) first assert `McpServerState` fails closed with `io_error` before returning early).
  - Verified unconditional `SKIP` returns in [`crates/pdf/tests/external_viewer_qa.rs:80-90, 297-304, 446-452, 521-529`](../../../../crates/pdf/tests/external_viewer_qa.rs#L80-L529) and [`crates/ui-egui/tests/macos_clipboard_live.rs:37-82`](../../../../crates/ui-egui/tests/macos_clipboard_live.rs#L37-L82), while [`.github/workflows/ci.yml`](../../../../.github/workflows/ci.yml) installs neither `pdftoppm` nor `gs`.

---

### Verdict `FA-X3-04` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` — Mock-only MCP tests assert drifted metric keys, `assert_contract` closes dialogs without post-patch readback, and `gui_wf` tests bypass broken `ChangeGraphType` via `graph.setSeries`.
- **Evidence:**
  - [`crates/mcp/tests/gui_only_surface.rs:553, 588`](../../../../crates/mcp/tests/gui_only_surface.rs#L553-L588) asserts `metrics.{width_px, height_px, scale_factor}` against `SharedMockHost` rather than production `ViewportMetrics` (`logical_width`/`logical_height`/`scale_factor`, related `FA-SEC21-01`).
  - [`crates/ui-egui/tests/audit_r2_s10.rs:182-195, 210`](../../../../crates/ui-egui/tests/audit_r2_s10.rs#L182-L210) patches `"duplicate.choice": "modify_existing"` and immediately closes the dialog without re-reading `ui.inspect` (`FA-SEC13-13`).
  - [`crates/ui-egui/tests/gui_wf_statistical_graphs.rs:344-346`](../../../../crates/ui-egui/tests/gui_wf_statistical_graphs.rs#L344-L346) and [`apps/curvebook-cli/tests/r73_mcp_gui_only_wf_16_28.rs:302-333`](../../../../apps/curvebook-cli/tests/r73_mcp_gui_only_wf_16_28.rs#L302-L333) explicitly bypass `run_graph_set_kind`'s discarded `option_id` by emitting direct `graph.setSeries` GDO patches.

---

### Verdict `FA-X3-05` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` — Release 1M-cell Go/No-Go gate is permanently `#[ignore]`d and excluded from `PERFORMANCE_FILTER`.
- **Evidence:**
  - [`crates/ui-egui/tests/r15_2_anova_and_1m_bench.rs:283-315, 540-542`](../../../../crates/ui-egui/tests/r15_2_anova_and_1m_bench.rs#L283-L542) gates `r15_2_gonogo_1m_column_matched_benchmark_release` with `#[ignore]` (documenting unresolved `views/grid_core.rs:323` clone latency), and neither [`.github/workflows/ci.yml`](../../../../.github/workflows/ci.yml) nor [`xtask/src/ci.rs`](../../../../xtask/src/ci.rs) invokes `--ignored`.
  - [`xtask/src/ci.rs:16`](../../../../xtask/src/ci.rs#L16) sets `PERFORMANCE_FILTER = "binary(r10_*) | binary(human_wf_*) | binary(*perf*)"`, excluding `r15_2_anova_and_1m_bench` so `r15_2_egui_1m_column_ci_smoke` runs in the parallel `tests` stage.

---

### Verdict `FA-X3-06` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` — `cargo xtask secrets` exits `1` when untracked non-ignored audit docs carry local paths, while `brands`/`cleanroom` skip `docs/audit/` and `ci policy` conditionally skips `full-gates` checks.
- **Evidence:**
  - Observed live execution of `cargo run --locked -q -p xtask -- secrets` returning `EXIT=1` (`496` local home-path exposure hits in working tree; `git grep -n '/Users/'` confirmed `0` real violations on tracked `main` files).
  - [`xtask/src/main.rs:172-190`](../../../../xtask/src/main.rs#L172-L190) (`repo_files` includes `--others --exclude-standard`) feeds [`xtask/src/secrets.rs:83`](../../../../xtask/src/secrets.rs#L83), whereas [`xtask/src/brands.rs:349-350`](../../../../xtask/src/brands.rs#L349-L350) and [`xtask/src/cleanroom.rs:195-196`](../../../../xtask/src/cleanroom.rs#L195-L196) skip `docs/audit/` completely; [`xtask/src/ci.rs:155-167`](../../../../xtask/src/ci.rs#L155-L167) and [`xtask/Cargo.toml:9-48`](../../../../xtask/Cargo.toml#L9-L48) gate `commands`/`docs-site` checks and tests on `full-gates`. (Distinct from `FA-SEC22-09` and `FA-SEC22-12`.)

---

### Verdict `FA-X3-07` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` — Vacuous `if let Some(...)` assertion blocks and discarded `execute_command` results silently skip checks when targets are missing.
- **Evidence:**
  - Verified unguarded `if let Some(...)` assertions in [`crates/ui-egui/tests/audit_r2_s10.rs:525-540, 547-553, 855-857`](../../../../crates/ui-egui/tests/audit_r2_s10.rs#L525-L857), [`r10_core_and_spike.rs:227-230`](../../../../crates/ui-egui/tests/r10_core_and_spike.rs#L227-L230), and [`r15_2_anova_and_1m_bench.rs:483-492`](../../../../crates/ui-egui/tests/r15_2_anova_and_1m_bench.rs#L483-L492), plus `let _ = host.execute_command("project.setTitle", ...)` followed by conditional `if host.app().document().title == "CON"` in [`apps/curvebook-web/tests/rv53_filename_sanitize.rs:57-61`](../../../../apps/curvebook-web/tests/rv53_filename_sanitize.rs#L57-L61).

---

### Verdict `FA-X3-08` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` — `human_wf_web_parity.rs` exercises headless engine commands (`ExecuteCommand`) and container headers rather than genuine web UI interactions or analysis results, and omits `WF-01..49`.
- **Evidence:**
  - [`apps/curvebook-web/tests/human_wf_web_parity.rs:202-411`](../../../../apps/curvebook-web/tests/human_wf_web_parity.rs#L202-L411): `web_human_r13_wf51_to_wf58_*` and `web_human_r13_wf59_to_wf65_*` route exclusively through `exec_web` (`WebBridgeEvent::ExecuteCommand`) plus one `ui.invoke_menu` call (`:263`), performing zero pointer/keyboard `UiInputEvent` dispatch, zero modal checks, and zero `ResultSheet` value/tab assertions (checking only `ResizeViewport`, ZIP `"PK"` bytes, and `"<svg"`). Confirmed `WF-01..49` have no test files under `apps/curvebook-web/tests/`.

---

### Verdict `FA-X3-09` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` — D1 edge gaps (`±Inf` absent across `crates/transform/tests/`, ties absent across `crates/fit/tests/`), `use_pratt_zeros = false` asserted in `stats`, and missing `is_dirty()` assertions on `project.open`/`save`.
- **Evidence:**
  - Independently scanned all `14` `crates/transform/tests/*.rs` (`0` contain `INFINITY`/`NEG_INFINITY`/`is_infinite`) and all `27` `crates/fit/tests/*.rs` (`0` cover predictor or replicate ties).
  - [`crates/stats/tests/audit_twogroup_col_fixes.rs:301`](../../../../crates/stats/tests/audit_twogroup_col_fixes.rs#L301) passes `false` (`use_pratt_zeros = false`) to `wilcoxon_matched_pairs` to agree with `run_analysis`'s defect (`FA-SEC05-06`).
  - Scanned all `35` `crates/engine/tests/*.rs`: [`audit_r1_s4b_eng.rs`](../../../../crates/engine/tests/audit_r1_s4b_eng.rs#L273-L296) and [`audit_r2_s05_s07.rs`](../../../../crates/engine/tests/audit_r2_s05_s07.rs#L145) assert `is_dirty()` around `sheet.select`/`table.setCell`/`table.convertArchetype`/`info.removeConstant`, while none of the `project.open`/`project.save` tests assert `session.is_dirty()` after open/save (`FA-SEC11-04`).

---

### Verdict `FA-X3-10` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S4` — Discarded `let _ = ...set_locale(...)` return values across UI tests mask `UiApp::set_locale`'s silent `"en"` fallback (`35` occurrences verified).
- **Evidence:**
  - [`crates/ui-egui/src/lib.rs:549-555`](../../../../crates/ui-egui/src/lib.rs#L549-L555) and [`crates/ui-egui/src/i18n.rs:204-233`](../../../../crates/ui-egui/src/i18n.rs#L204-L233) normalize unknown locale tags silently to `"en"`.
  - Automated scan across `crates/ui-egui/tests/` confirmed **35** `let _ = ...set_locale(...)` sites (including all cited sites in `human_wf_common/mod.rs:310,316`, `r15_2_anova_and_1m_bench.rs:595`, `cjk_rtl_hidpi_desktop_qa.rs:166,401`, `r10_23_beta_desktop_gaps.rs:291`, `r10_phase7_gates.rs:405,413`, `r15_4_r10_23_egui_gaps.rs:322,411,439,582`), plus [`audit_r2_s10.rs:683`](../../../../crates/ui-egui/tests/audit_r2_s10.rs#L683) discarding `handle_control_line` for `ui.locale.set`.
