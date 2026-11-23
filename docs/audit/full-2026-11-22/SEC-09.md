# Security & Correctness Audit — SEC-09: Chart Compiler & Scales (`crates/chart`)

- **Base SHA:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Scope:** `SEC-09` — Chart Compiler & Scales (`crates/chart/Cargo.toml` and all 9 files in `crates/chart/src/`)
- **Date:** `2026-11-22`
- **Summary Counts:** `S1 = 5`, `S2 = 6`, `S3 = 1`, `S4 = 1` (**Total: 13 findings**)

---

## File Coverage Table

| File | Exact Lines | Lines Inspected | Concrete Invariants Checked | Finding IDs / Status |
|---|---:|---|---|---|
| `crates/chart/Cargo.toml` | 26 | `1..=26` | Workspace lint inheritance (`[lints] workspace = true`), `wasm32` dependency compatibility (no `chrono` or native-only crates), unused dependencies | `FA-SEC09-13` |
| `crates/chart/src/lib.rs` | 207 | `1..=207` | Crate `#![forbid(unsafe_code)]`, canvas/page finite positive bounds via `clamp_f64_safe`, cell marker override resolution, `compile_graph` / `compile_layout` dispatch | Clean — verified `#![forbid(unsafe_code)]`, NaN/inverted dimension guards in `clamp_f64_safe`, deterministic marker override precedence, and valid compiler routing |
| `crates/chart/src/scene.rs` | 482 | `1..=482` | `ChartScene` IR invariants, `RectPt::new` finite/non-negative sizing, `hit_test_point`, multi-line `text_primitive_bounds_pt` (`split_rich_text_lines`), `preflight_check_scene` | Clean — verified `ChartScene`, `RectPt::new`, `hit_test_point`, `text_primitive_bounds_pt`, and `preflight_check_scene` guard against non-finite coords/sizes and overflows |
| `crates/chart/src/image_decode.rs` | 189 | `1..=189` | Per-asset decode caps (`MAX_IMAGE_ASSET_BYTES`, `MAX_IMAGE_DIM_PX`, `MAX_IMAGE_PIXELS * 16` via `bounded_reader`), magic-byte sniff + MIME check in `decode_asset_rgba8`, `emit_embedded_image_primitive` | Clean per-asset decode bounds; cumulative budget (`pixel_budget = None`) bypassed by graph callers (`FA-SEC09-12`) |
| `crates/chart/src/equation_box.rs` | 2595 | `1..=2595` | 2D TeX AST parser termination (`parse_math_slice`, `parse_next_step`, `attach_scripts_if_any`, `parse_script_arg`), Knuth layout bounds, `resolve_model_equation`, `render_model_equation_annotation`, hit-test bounds | `FA-SEC09-01` |
| `crates/chart/src/scales.rs` | 3851 | `1..=3851` | Scale forward/inverse transforms (`scale_transform`, `scale_inverse`, `AxisMapping::map_value`), broken-axis segment splitting, domain/tick resolution (`resolve_sample_segment_domain`, `resolve_exact_domain_with_warnings`, `sanitize_linear_range`), hooks, chrome & annotation image budget | `FA-SEC09-03`, `FA-SEC09-12`, `FA-SEC09-13` |
| `crates/chart/src/xy.rs` | 3111 | `1..=3111` | `SegmentedAxis`, `expand_doc_text`, `configure_plot_rect`, `summarize_subcells`, `render_cartesian_chrome_full`, `render_legend_and_annotations`, `render_styled_embedded_table`, `compile_xy_graph`, regression curve resampling | `FA-SEC09-08`, `FA-SEC09-10`, `FA-SEC09-11`, `FA-SEC09-12`, `FA-SEC09-13` |
| `crates/chart/src/column_grouped.rs` | 2723 | `1..=2723` | Vertical/horizontal dispatch (`compile_column_or_grouped_graph`, `compile_horizontal_column_or_grouped`), Column/Nested/Grouped extraction, categorical tick source resolution, before-after connectors, box/violin/floating/bar geometry, `build_kde_profile` | `FA-SEC09-07`, `FA-SEC09-08`, `FA-SEC09-11` |
| `crates/chart/src/survival_cat.rs` | 2459 | `1..=2459` | KM cohorts (`compile_survival`, `build_km_from_result_sheet`, censoring markers), contingency stacked/superimposed bars & parts-of-whole, scalar/info hooks (`resolve_graph_text`, `resolve_result_scalar`), frame/legend/colorbar/annotations, `extract_sheet_grid` | `FA-SEC09-06`, `FA-SEC09-09`, `FA-SEC09-11`, `FA-SEC09-12`, `FA-SEC09-13` |
| `crates/chart/src/multivariate_layout.rs` | 4335 | `1..=4335` | `BubbleScatter`, `CategoricalSummary`, `Heatmap`, `Volcano`, `BlandAltman`, `RocCurve`, `ForestPlot`, `ResidualPlot`, `NormalQqPlot`, `collect_ds_active_values_with_sources`, `find_matching_result`, multi-panel `compile_layout_scene` & `transform_primitive` | `FA-SEC09-02`, `FA-SEC09-04`, `FA-SEC09-05`, `FA-SEC09-06`, `FA-SEC09-10`, `FA-SEC09-11`, `FA-SEC09-12` |

**Total Audited:** 19,978 lines (`crates/chart/Cargo.toml`: 26 + `crates/chart/src/*.rs`: 19,952), `100%` (`1..=N`) across all 10 files.

---

## Findings

### FA-SEC09-01 (S1) — Infinite parser loop & OOM abort on malformed TeX script syntax in `equation_box.rs`
- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/chart/src/equation_box.rs:681-703, 758-763, 1040-1044, 1061-1070`
- **Category:** `no-panic` / `resources`
- **Defect:** In `parse_next_step`, encountering a leading `'^'` or `'_'` without advancing `pos` delegates to `attach_scripts_if_any` with an empty `MathNode::Group(Vec::new())` nucleus. Inside `attach_scripts_if_any`, when `parse_script_arg` returns `None` (e.g. end-of-input, unclosed `'{'`, or unmatched `'}'`), `attach_scripts_if_any` executes `break;` without advancing `*pos`. `parse_math_slice` then loops at the exact same `pos` forever while pushing `MathNode::Group(Vec::new())` into `nodes` until memory exhaustion aborts the process.
- **Failure scenario:** Any model equation DSL/custom TeX string containing `"x^"`, `"x_ "`, `"y = ^"`, `"y = ^{unclosed"`, or `"^}"` triggers an immediate hang followed by an OOM abort during graph or layout compilation (affecting both GUI preview and headless SVG/EMF/PDF export).
- **Evidence:**
  ```rust
  // crates/chart/src/equation_box.rs:681-703
      while pos < input.len() {
          ...
          if let Some(step) = parse_next_step(input, &mut pos, text_mode, depth) {
              match step {
                  ParsedStep::Raw(node) => nodes.push(node),
  ```
  ```rust
  // crates/chart/src/equation_box.rs:758-763
      if ch == '^' || ch == '_' {
          // Standalone script marker without a preceding nucleus: attach to an empty nucleus.
          let empty = MathNode::Group(Vec::new());
          let attached = attach_scripts_if_any(empty, input, pos, text_mode, depth);
          return Some(ParsedStep::Raw(attached));
      }
  ```
  ```rust
  // crates/chart/src/equation_box.rs:1040-1044
          let after = rest.get(1..).unwrap_or("");
          let Some((arg_nodes, arg_consumed)) = parse_script_arg(after, text_mode, depth + 1) else {
              break;
          };
          *pos += 1 + arg_consumed;
  ```
  ```rust
  // crates/chart/src/equation_box.rs:1061-1070
  fn parse_script_arg(s: &str, text_mode: bool, depth: usize) -> Option<(Vec<MathNode>, usize)> {
      let trimmed = s.trim_start_matches([' ', '\t']);
      ...
      let first = trimmed.chars().next()?;
      if first == '{' {
          let (inner, consumed) = extract_braced(trimmed)?;
  ```
- **Suggested fix:** Require progress in `parse_next_step`: when `ch == '^' || ch == '_'`, pass `&mut pos` after checking `parse_script_arg`; if `attach_scripts_if_any` does not advance `pos`, consume the orphan `'^'`/`'_'` character as a literal `MathNode` so `pos` strictly increases on every `parse_math_slice` iteration.
- **Related:** None.

---

### FA-SEC09-02 (S1) — `CategoricalSummary` plots wrong 95% CIs, silent geometric-mean fallbacks, truncated error-bar domains, and dropped style/hit-test state
- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/chart/src/multivariate_layout.rs:612, 655-679, 744, 764-770, 779-797, 988-991, 1004-1022`
- **Category:** `wrong-result`
- **Defect:** `compile_categorical_summary` diverges statistically and visually from `xy.rs` / `column_grouped.rs`:
  1. `ErrorBarMetric::ConfidenceInterval95` uses a hardcoded normal multiplier `1.96 * sd / sqrt(n)` instead of exact Student-$t$ critical values $t_{0.975, n-1}$ (noticeably under-estimating CI width for small $n$, and failing to check $n < 2$).
  2. `summary_center` silently falls back to the arithmetic mean when `SummaryMeasure::GeometricMean` sees zero or negative values, whereas `xy.rs:203-215` emits a `non_positive_geometric_mean` warning and omits the undefined summary.
  3. Automatic domain computation (`data_lo`/`data_hi`) considers raw observation values only and ignores upper/lower `y_error_bars` bounds, clipping error bars out of the plot frame.
  4. `emit_error_bar` draws horizontal caps even when `eb.cap_width_pt <= 0.0`, and silently drops the entire error bar if either endpoint fails `map_value` (e.g., negative lower bound on a log axis).
  5. Connecting lines (`BeforeAfterLines` / `ScatterAndLine`) hardcode `palette.color_at(0)` ignoring `GraphicalDataObject` stroke styles; `emit_replicate_dots` ignores per-cell `point_overrides` and never populates `scene.plotted_points` for interactive hit-testing.
- **Failure scenario:** Any `CategoricalSummary` graph on a `MultipleVariables` sheet with small $n$ ($n=3$: `1.96` vs `4.303`, >2x too narrow CI), non-positive values under geometric mean, or tall error bars renders scientifically inaccurate intervals/means without warning and loses click hit-testing.
- **Evidence:**
  ```rust
  // crates/chart/src/multivariate_layout.rs:612 & 988-991
              curvebook_doc::ErrorBarMetric::ConfidenceInterval95 => center_y + 1.96 * sd / (values.len().max(1) as f64).sqrt(),
  ...
          curvebook_doc::ErrorBarMetric::ConfidenceInterval95 => {
              let half = 1.96 * sd / (n as f64).sqrt();
              (center - half, center + half)
          }
  ```
  ```rust
  // crates/chart/src/multivariate_layout.rs:764-770 (vs xy.rs:203-215)
          curvebook_doc::SummaryMeasure::GeometricMean => {
              if values.iter().all(|&v| v > 0.0) {
                  (values.iter().map(|v| v.ln()).sum::<f64>() / (values.len() as f64)).exp()
              } else {
                  values.iter().sum::<f64>() / (values.len().max(1) as f64)
              }
          }
  ```
  ```rust
  // crates/chart/src/multivariate_layout.rs:655-679 & 1004-1022
      let mut data_lo = f64::INFINITY;
      let mut data_hi = f64::NEG_INFINITY;
      for g in &groups {
          for &v in g {
              data_lo = data_lo.min(v);
              data_hi = data_hi.max(v);
          }
      }
  ...
      let (Some(y_lo), Some(y_hi)) = (y_map.map_value(y_lo_val, clip.bottom(), clip.y), y_map.map_value(y_hi_val, clip.bottom(), clip.y)) else {
          return;
      };
  ...
      let half_cap = eb.cap_width_pt * 0.5;
      for y_cap in [y_lo, y_hi] {
  ```
- **Suggested fix:** Reuse `xy::compute_central_tendency`, `xy::compute_raw_error_bounds` (exact Student-$t$ quantile), and `xy::emit_directional_error_bar`; include error-bar extents in `data_lo`/`data_hi`; guard `eb.cap_width_pt > 0.0`; apply per-cell `point_overrides` and push `plotted_points`.
- **Related:** `FA-SEC09-08`.

---

### FA-SEC09-03 (S1) — Log/explicit/extreme axis domain resolution in `scales.rs` produces inverted domains, drops small log spans, and breaks mapping
- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/chart/src/scales.rs:189-198, 1185-1191, 1246-1253, 1303-1365`
- **Category:** `wrong-result`
- **Defect:** Four domain-resolution defects corrupt axis mapping in `scales.rs`:
  1. `resolve_exact_domain_with_warnings` repairs non-positive `lo <= 0.0` on `Log10`/`Log2`/`Ln` to `0.1` when `raw_lo <= 0.0`, without checking `lo >= hi`. When `hi < 0.1` (e.g., data in `[0.0, 0.05]`), it returns an inverted domain `(0.1, 0.05)` instead of scaling relative to `hi`.
  2. `resolve_exact_domain_with_warnings` checks `(hi - lo).abs() <= 1e-12` in linear space across `Log10`/`Log2`/`Ln`, `Logit`, `Probit`, and `Reciprocal` axes (`lines 1309, 1330, 1342, 1357`), rejecting valid multi-decade sub-picomolar log domains (e.g., `[1e-14, 1e-12]`) when repaired and replacing them with `sample_default_domain`.
  3. Single-sided explicit bounds (`explicit_min > raw_hi` or `explicit_max < raw_lo`) in `resolve_exact_domain_with_warnings` silently produce inverted `lo > hi`; and in `resolve_sample_segment_domain` (`lines 1185-1191`), `extend_upper_bound`/`extend_lower_bound` only runs when `seg_valid.is_empty()`.
  4. `sanitize_linear_range` (`lines 1248-1253`) on finite extreme inputs like `(-1e308, 1e308)` checks `!span.is_finite()`, which overflows to `+Inf`, then widens to `(lo - pad, hi + pad)` = `(-1.1e308, 1.1e308)` — whose `denom = t_max - t_min` in `AxisMapping::map_value` is still `+Inf`, causing `map_value` (`lines 192-195`) to return `None` for every data point.
- **Failure scenario:** Dose-response plots in femtomolar/picomolar concentration units (`1e-14..1e-12` M), fractional log plots in `(0, 0.05]`, user-pinned `min > auto_max`, or extreme-magnitude linear data produce empty or inverted charts.
- **Evidence:**
  ```rust
  // crates/chart/src/scales.rs:1185-1191
      if seg_valid.is_empty() {
          if explicit_min.is_some() && explicit_max.is_none() && dmax <= dmin {
              dmax = extend_upper_bound(dmin, scale);
          } else if explicit_max.is_some() && explicit_min.is_none() && dmin >= dmax {
              dmin = extend_lower_bound(dmax, scale);
          }
      }
  ```
  ```rust
  // crates/chart/src/scales.rs:1246-1253 & 192-195
      let (lo, hi) = if min_v <= max_v { (min_v, max_v) } else { (max_v, min_v) };
      let span = hi - lo;
      if !span.is_finite() || span.abs() <= MIN_FINITE_AXIS_SPAN {
          let pad = linear_constant_pad(lo);
          (lo - pad, hi + pad)
      } else {
          (lo, hi)
      }
  // AxisMapping::map_value (scales.rs:192-195)
          let denom = t_max - t_min;
          if !denom.is_finite() || denom.abs() <= MIN_FINITE_AXIS_SPAN {
              return None;
          }
  ```
  ```rust
  // crates/chart/src/scales.rs:1303-1334
      let mut lo = explicit_min.unwrap_or(raw_lo);
      let mut hi = explicit_max.unwrap_or(raw_hi);
  ...
                  if lo <= 0.0 {
                      lo = if raw_lo > 0.0 && scale_transform(scale, raw_lo).is_some() { raw_lo } else { 0.1 };
                  }
  ...
                  if (hi - lo).abs() <= 1e-12 || scale_transform(scale, lo).is_none() || scale_transform(scale, hi).is_none() {
                      return sample_default_domain(scale);
                  }
  ```
- **Suggested fix:** Repair log `lo <= 0.0` relative to `hi` (`(hi * 0.01).min(hi * 0.5)`) rather than hardcoded `0.1`; check degenerate span in transformed space `(t_hi - t_lo).abs() <= MIN_FINITE_AXIS_SPAN`; apply `extend_upper_bound`/`extend_lower_bound` on single-sided explicit bounds regardless of `seg_valid.is_empty()`; clamp linear bounds symmetrically only when span is zero rather than widening already-finite `(-1e308, 1e308)` into `Inf` span (use normalized `(v - mid) / half_span` or clamp to safe range in `map_value`).
- **Related:** None.

---

### FA-SEC09-04 (S1) — Bland-Altman extraction ignores `xy_compare_x_vs_y`, mislabels LoA captions, and overrides columns on `MultipleVariables`
- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/chart/src/multivariate_layout.rs:2517-2519, 2596-2627, 2635-2639`
- **Category:** `wrong-result` / `contract-drift`
- **Defect:** `extract_bland_altman_pairs` exhibits three contract mismatches against `BlandAltmanSpec` (`crates/doc/src/analysis.rs`) and `curvebook-stats`:
  1. On `TableArchetype::Xy(xy)`, it unconditionally pairs X vs Y of `col_a` via `active_xy_pairs_with_sources(&ds.id, xy, col_a)`, completely ignoring `ba.xy_compare_x_vs_y` (which selects comparing `Y[column_a]` vs `Y[column_b]` when `xy.data_sets.len() >= 2` and `xy_compare_x_vs_y != Some(true)`).
  2. When raw data is extracted on `TableArchetype::MultipleVariables(mv)`, `graph.spec.multivariate.as_ref().map_or(col_a, |m| m.x_column)` and `m.y_column` override the `BlandAltmanSpec` `column_a`/`column_b` resolved at lines `2638-2639`.
  3. Reference line annotations unconditionally format `"+1.96 SD: {loa_hi:.2}"` / `"-1.96 SD: {loa_lo:.2}"` even when non-default LoA multipliers or non-parametric quantiles are shown from a linked `ResultSheet`.
- **Failure scenario:** Plotting a Bland-Altman graph from a two-dataset XY sheet (`Method A` in Y0, `Method B` in Y1) silently computes `X - Y0` instead of `Y0 - Y1`, producing a completely meaningless difference plot relative to the companion statistical result sheet.
- **Evidence:**
  ```rust
  // crates/chart/src/multivariate_layout.rs:2596-2627
          TableArchetype::MultipleVariables(mv) if mv.variables.len() >= 2 => {
              let xi = graph.spec.multivariate.as_ref().map_or(col_a, |m| m.x_column);
              let yi = graph.spec.multivariate.as_ref().map_or(col_b, |m| m.y_column);
  ...
          TableArchetype::Xy(xy) => {
              let pairs = active_xy_pairs_with_sources(&ds.id, xy, col_a);
              let label = xy.data_sets.get(col_a).map_or_else(|| String::from("Method A"), |d| d.title.clone());
  ```
  ```rust
  // crates/chart/src/multivariate_layout.rs:2517-2519
          (loa_hi, solid_stroke(Rgba8::rgb(213, 94, 0), 1.2, LineDashStyle::Dashed), format!("+1.96 SD: {loa_hi:.2}")),
          (loa_lo, solid_stroke(Rgba8::rgb(213, 94, 0), 1.2, LineDashStyle::Dashed), format!("-1.96 SD: {loa_lo:.2}")),
  ```
- **Suggested fix:** Thread `xy_compare_x_vs_y` from `AnalysisKind::BlandAltman(ba)` (defaulting to `xy.data_sets.len() == 1` when `None`) and pair `xy.data_sets[col_a]` vs `xy.data_sets[col_b]` row-wise when comparing Y datasets; preserve `ba.column_a`/`ba.column_b` when linked to a `BlandAltman` analysis; and reflect configured LoA multiplier/bounds in LoA labels.
- **Related:** `curvebook-stats` (`two_group.rs`).

---

### FA-SEC09-05 (S1) — Pre-summarized `Grouped`/`Column` subcolumns (`SD`, `SEM`, `N`, `IQR`) are averaged as raw replicates in Heatmaps, Normal Q-Q, and Residual plots
- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/chart/src/multivariate_layout.rs:1147-1164, 1742-1750, 3349-3359, 3675`
- **Category:** `wrong-result`
- **Defect:** Both `Grouped` Heatmap extraction (`lines 1742-1750`) and `collect_ds_active_values_with_sources` (`lines 1147-1164`, used by Residual and Normal Q-Q fallback paths) iterate all subcolumns indiscriminately without checking `table.subcolumn_mode.is_pre_summarized()`. On `MeanSdN`, `MeanSemN`, `MedianIqr`, or `UpperLowerLimits` tables, `Mean` (subcol 0), `SD`/`SEM` (subcol 1), and `N` (subcol 2) are pooled/averaged as replicate observations.
- **Failure scenario:** A Heatmap built from a `Grouped(MeanSdN)` table with `Mean=10, SD=2, N=6` renders cell value `(10 + 2 + 6) / 3 = 6.0` instead of `10.0`; Normal Q-Q and Residual plots on pre-summarized `Column` tables treat `SD` and `N` as observations.
- **Evidence:**
  ```rust
  // crates/chart/src/multivariate_layout.rs:1742-1750
                          .map(|&c| {
                              let reps: Vec<f64> = g.active_replicates(r, c).into_iter().filter(|x| x.is_finite()).collect();
                              if reps.is_empty() {
                                  None
                              } else {
                                  let mean = reps.iter().sum::<f64>() / (reps.len() as f64);
                                  mean.is_finite().then_some(mean)
                              }
                          })
  ```
  ```rust
  // crates/chart/src/multivariate_layout.rs:1150-1164
          TableArchetype::Column(col) => {
              let c = col_idx.min(col.data_sets.len().saturating_sub(1));
              let label = col.data_sets.get(c).map_or_else(|| format!("Group {}", c + 1), |d| d.title.clone());
              if let Some(d) = col.data_sets.get(c) {
                  for r in 0..col.row_count() {
                      if let Some(row_cells) = d.cells.get(r) {
                          for (subcol, cell) in row_cells.iter().enumerate() {
                              if let Some(v) = cell.active_number().filter(|x| x.is_finite()) {
                                  out.push((v, SourceCellId { sheet_id: ds.id.clone(), row: r, col: c, subcol }));
                              }
                          }
                      }
                  }
              }
  ```
- **Suggested fix:** Check `subcolumn_mode.is_pre_summarized()` in both Heatmap grouped extraction and `collect_ds_active_values_with_sources`, extracting only the central summary subcolumn (`subcol == 0`) or emitting a diagnostic warning when replicate-only plots are bound to pre-summarized tables.
- **Related:** `FA-SEC09-08`.

---

### FA-SEC09-06 (S2) — `survival_cat.rs` and `multivariate_layout.rs` ignore `ResultStatus::Failed` and fall back to mismatched scalar datasets
- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/chart/src/survival_cat.rs:141-167, 1016-1029, 1241-1266` and `crates/chart/src/multivariate_layout.rs:1029-1043, 2635-2636`
- **Category:** `contract-drift` / `wrong-result`
- **Defect:** Unlike `equation_box::resolve_model_equation` (`equation_box.rs:182-188`), `column_grouped::resolve_brackets` (`column_grouped.rs:2156-2162`), and `ResultSheet::lookup_scalar` (`doc`), `survival_cat.rs` and `multivariate_layout.rs` consume `ResultSheet`s without inspecting `res.status`:
  1. `survival_cat::resolve_result_scalar` (`lines 141-166`) reads `res.scalars` directly when `res.status` is `Failed`/`Dirty`, and at `line 159` falls back to any dataset's scalar with the same key when an explicit `data_set_index` is missing.
  2. `survival_cat::extract_sheet_grid` (`lines 1016-1029`) renders stale/empty tables from failed `ResultSheet` tabs or scalars (also driving `EmbeddedResultTable` and `LiveTable` in `xy.rs`).
  3. `survival_cat::extract_km_cohorts` (`lines 1243-1265`) and `multivariate_layout::{find_matching_result, matching_results}` (`lines 1029-1043`) accept failed `ResultSheet`s with zero warning instead of falling back or reporting failure.
- **Failure scenario:** After an analysis edit causes `ResultSheet.status = Failed`, graphs and layouts silently keep rendering stale fitted curves, tables, or wrong-dataset scalar text hooks (`{{result:res1:2:HazardRatio}}` printing dataset 0's value).
- **Evidence:**
  ```rust
  // crates/chart/src/survival_cat.rs:141-166
  fn resolve_result_scalar(res: &ResultSheet, key: &str, data_set_index: Option<usize>) -> Option<String> {
      let needle = key.trim();
      if let Some(entry) =
          res.scalars.iter().find(|s| s.key.trim().eq_ignore_ascii_case(needle) && (data_set_index.is_none() || s.data_set_index == data_set_index))
      {
          ...
          return Some(s);
      }
      res.scalars.iter().find(|s| s.key.trim().eq_ignore_ascii_case(needle)).and_then(|entry| {
  ```
  ```rust
  // crates/chart/src/survival_cat.rs:1016-1023
  pub(crate) fn extract_sheet_grid(doc: &ProjectDocument, source_sheet_id: &SheetId, tab_id: Option<&str>) -> Option<SheetGrid> {
      if let Some(res) = doc.result_sheet(source_sheet_id) {
          let tab: Option<&ResultTab> = tab_id.and_then(|tid| res.tabs.iter().find(|t| t.id == tid)).or_else(|| res.tabs.first());
  ```
  ```rust
  // crates/chart/src/multivariate_layout.rs:1029-1043
  fn find_matching_result<'a>(graph: &GraphSheet, doc: &'a ProjectDocument) -> Option<&'a ResultSheet> {
      doc.result_sheet(&graph.spec.primary_source_sheet_id)
          .or_else(|| doc.result_sheets.iter().find(|r| r.analysis.source_sheet_id == graph.spec.primary_source_sheet_id))
  }
  ```
- **Suggested fix:** Gate `resolve_result_scalar`, `extract_sheet_grid`, `extract_km_cohorts`, and `matching_results` on `!matches!(res.status, ResultStatus::Failed { .. })`, emit diagnostic placeholders/warnings for failed sheets, and remove the cross-dataset fallback in `resolve_result_scalar` when `data_set_index.is_some()`.
- **Related:** `FA-SEC09-11`.

---

### FA-SEC09-07 (S2) — Horizontal column/grouped mode and Grouped `BeforeAfterLines` silently drop series geometry
- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/chart/src/column_grouped.rs:207, 227, 238, 242, 271-275, 1114-1116, 1199`
- **Category:** `wrong-result` / `contract-drift`
- **Defect:** `compile_horizontal_column_or_grouped` omits major chart features supported by the vertical path:
  1. `render_horizontal_item` (`lines 271-275`) handles `BarWithReplicates`, `AlignedDotPlot`, `ScatterAndLine`, and `FloatingBar`, then silently ignores `BoxAndWhisker`, `Violin`, and `BeforeAfterLines` (`=> {}`) with no warning.
  2. Horizontal compilation hardcodes `y2: None` (`line 238`), ignoring `graph.spec.y2_axis` and `bind_to_right_y2`, and skips CLD letters and significance brackets completely.
  3. On vertical graphs, `render_before_after_connectors` (`lines 1114-1116`) early-returns unless `TableArchetype::Column`, and `render_vertical_item` returns `None` (`line 1199`), silently leaving `BeforeAfterLines` series blank on `GroupedTable`.
- **Failure scenario:** Switching a box-plot, violin plot, or before-after graph to `CartesianOrientation::Horizontal`, or rendering `BeforeAfterLines` on a `Grouped` table, outputs an empty Cartesian frame without any chart warning.
- **Evidence:**
  ```rust
  // crates/chart/src/column_grouped.rs:238-246 & 271-276
      render_cartesian_chrome_full(scene, graph, doc, &ChromeAxes { x: &x_axis, y: &y_axis, y2: None, cat_x: None, cat_y: cat_ticks_opt });

      let mut stacks = StackAccum::default();
      for item in &mut items {
          render_horizontal_item(scene, &graph.spec, item, &x_axis, &y_axis, &mut stacks);
      }
  ...
          GraphicalDataObject::BeforeAfterLines { .. }
          | GraphicalDataObject::BoxAndWhisker { .. }
          | GraphicalDataObject::Violin { .. }
          | GraphicalDataObject::KaplanMeierStep { .. }
          | GraphicalDataObject::PartsOfWhole { .. } => {}
  ```
  ```rust
  // crates/chart/src/column_grouped.rs:1114-1116 & 1198-1200
      let TableArchetype::Column(col_table) = &sheet.archetype else {
          return;
      };
  ...
          GraphicalDataObject::BeforeAfterLines { .. } => None,
  ```
- **Suggested fix:** Implement horizontal box-whisker, violin, and before-after renderers (or emit an explicit unsupported-orientation warning when unimplemented) and extend `render_before_after_connectors` to pair row/replicate values on `GroupedTable`.
- **Related:** None.

---

### FA-SEC09-08 (S2) — Pre-summarized box/violin collapse, unsorted pre-summarized hinges, silent log clamping, and reversed-axis KDE collapse in `column_grouped.rs`
- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/chart/src/column_grouped.rs:771, 964, 1418-1438, 1569-1587, 1682-1690, 1749-1753` and `crates/chart/src/xy.rs:83-106, 241-244`
- **Category:** `wrong-result`
- **Defect:** Multiple geometric/domain bugs affect distribution and pre-summarized items:
  1. `extract_column_presummarized` (`line 771`) and `push_grouped_cell` (`line 964`) call `xy::summarize_subcells(..., y_error_bars.as_ref(), warnings)`. Because `BoxAndWhisker` / `Violin` / `FloatingBar` commonly carry `y_error_bars: None`, `summarize_pre_summarized` (`xy.rs:242-244`) ignores subcolumns 1 and 2 and returns `(center, center, center)`, flattening boxes, violins, and floating bars to zero height.
  2. Even when `y_error_bars` is set, `render_box_whisker_item` (`lines 1420-1424`) and `render_violin_item` (`lines 1571-1575`) build `vec![lower, center, upper]` without sorting before calling `quartiles_hinge(&sorted)`. If `center < lower` or `upper < center` (e.g., inverted explicit limits), `quartiles_hinge` treats the slice as sorted and computes invalid quartiles.
  3. `render_box_whisker_item` (`lines 1433-1437`) and `render_floating_bar_item` (`lines 1749-1750`) map endpoints with `y_axis.map_clamped_to_nearest(...)` and discard the `.2` `clipped` boolean, silently clamping non-positive log-axis bounds onto `domain_min` with no warning.
  4. `build_kde_profile` (`lines 1682-1690`) clamps `hi = hi.min(seg.domain_max)` directly against raw `domain_max`. On a reversed `y_axis` (`domain_min > domain_max`, e.g., `[100.0, 0.0]` or `[100.0, 1.0]`), `hi <= lo` triggers `return Vec::new()`, reducing violins to a single median bar.
- **Failure scenario:** Plotting a `MeanSdN` or `UpperLowerLimits` table as a Box-and-Whisker / Violin / FloatingBar collapses every glyph to a horizontal tick, or reversing the Y axis turns Violin plots into flat ticks.
- **Evidence:**
  ```rust
  // crates/chart/src/xy.rs:241-244
      let center_only = subcells.first().and_then(curvebook_doc::Cell::active_number).filter(|v| v.is_finite());
      let Some(eb) = error_spec else {
          return center_only.map(|c| (c, c, c));
      };
  ```
  ```rust
  // crates/chart/src/column_grouped.rs:1418-1437
      let sorted_obs = match &item.payload {
          ExtractedKind::Raw(obs) => sorted_finite_obs(obs),
          ExtractedKind::PreSummarized { center, lower, upper, source } => vec![
              RawObs { value: *lower, source: source.clone() },
              RawObs { value: *center, source: source.clone() },
              RawObs { value: *upper, source: source.clone() },
          ],
      };
  ...
      let (_, py_q1, _) = y_axis.map_clamped_to_nearest(q1);
  ```
  ```rust
  // crates/chart/src/column_grouped.rs:1682-1690
      if let Some(seg0) = y_axis.segments.first() {
          if is_log_like(seg0.scale) {
              lo = lo.max(seg0.domain_min.max(min_v * 0.85));
          }
          hi = hi.min(y_axis.segments.last().map_or(hi, |s| s.domain_max));
      }
      if hi <= lo {
          return Vec::new();
      }
  ```
- **Suggested fix:** Derive fallback bounds directly from `subcolumn_mode` (`UpperLowerLimits` / `MeanSdN`) when `y_error_bars` is `None`; sort pre-summarized `[lower, center, upper]` before `quartiles_hinge`; use `domain_min.min(domain_max)` / `domain_min.max(domain_max)` in `build_kde_profile`; warn when clamping out-of-domain log values in box/floating plots.
- **Related:** `FA-SEC09-02`, `FA-SEC09-05`.

---

### FA-SEC09-09 (S2) — Survival KM result-sheet extraction emits false censoring ticks, propagates non-finite coordinates, and drops stacked contingency slices on log axes
- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/chart/src/survival_cat.rs:1151-1155, 1480-1496, 1549-1559, 1903-1919`
- **Category:** `wrong-result`
- **Defect:** Survival and Contingency compilation contain four correctness defects:
  1. `extract_censored_from_nar_tab` (`lines 1551-1557`) has an overly broad `else if ev_now == 0 && r_now > 0` branch that marks a censoring tick at every at-risk-table row with zero events — including $t = 0$ and scheduled follow-up rows where zero subjects were censored (`prev == r_now`).
  2. `build_km_from_result_sheet` (`lines 1480-1496`) copies `p.x`, `p.y`, `p.ci_lower`, `p.ci_upper` without filtering non-finite values; `max_time` (`lines 1151-1155`) uses `f64::max` over steps and censored times, so a single `f64::INFINITY` collapses `x_axis` to `[0, +Inf]` (while `f64::max` skips `NaN` in `fold`, leaving non-finite `NaN` steps in `steps`).
  3. In `compile_contingency` (`lines 1903-1919`), stacked bars test `(y_map.map_value(top_val, ...), y_map.map_value(acc, ...))` with `else { acc = top_val; continue; }`. On a log Y scale where `acc = 0.0` at the base slice (`s_idx = 0`), `map_value(0.0, ...)` returns `None`, silently dropping the bottom bar slice without warning.
- **Failure scenario:** A Kaplan-Meier curve reconstructed from a `ResultSheet` displays spurious censoring ticks at $t=0$ and at every non-event follow-up checkpoint; a stacked Contingency bar chart with a `Log10` Y axis silently loses its bottom category slice.
- **Evidence:**
  ```rust
  // crates/chart/src/survival_cat.rs:1549-1559
          let r_now = (*risk).max(0.0) as usize;
          let ev_now = (*events).max(0.0) as usize;
          if let Some(prev) = prev_risk
              && prev > r_now + ev_now
          {
              out.push((*row_t, None, col_idx));
          } else if ev_now == 0 && r_now > 0 {
              out.push((*row_t, None, col_idx));
          }
          prev_risk = Some(r_now);
  ```
  ```rust
  // crates/chart/src/survival_cat.rs:1903-1909
              let mut acc = 0.0;
              for (s_idx, &val) in row_vals.iter().enumerate() {
                  let top_val = acc + val;
                  let (Some(y_top), Some(y_bot)) = (y_map.map_value(top_val, r.bottom(), r.y), y_map.map_value(acc, r.bottom(), r.y)) else {
                      acc = top_val;
                      continue;
                  };
  ```
- **Suggested fix:** Only emit censoring ticks in `extract_censored_from_nar_tab` when `prev > r_now + ev_now` (strictly losing subjects beyond observed events); filter `!p.x.is_finite() || !p.y.is_finite()` in `build_km_from_result_sheet` and finite-guard `max_time`; anchor stacked bar base `acc = 0.0` to `r.bottom()` or warn when `Log` Y-axis is applied to zero-based stacked bars.
- **Related:** None.

---

### FA-SEC09-10 (S2) — Forest plot right-margin CI clipping & log-row dropping; reciprocal XY curve resampling range bug
- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/chart/src/multivariate_layout.rs:2974, 3016-3022, 3054-3061, 3133-3136` and `crates/chart/src/xy.rs:2714-2723`
- **Category:** `wrong-result`
- **Defect:** Two independent projection bugs truncate or drop valid geometry:
  1. In `compile_forest_plot` (`multivariate_layout.rs:2974`), right-side margin space for the tabular `"{estimate:.2} [{lower:.2}, {upper:.2}]"` labels at `r.right() + 6.0` is only reserved via `has_legend: true` — which `update_plot_rect_for_labels` (`survival_cat.rs:247`) grants only when `spec.legend.visible && spec.legend.position_pt.is_none()`. Hiding the legend shrinks right margin to `18 pt` and clips CI text off the canvas. Furthermore, lines `3016-3022` `continue` out of the loop whenever any of `(lower, estimate, upper)` fails `map_value` (e.g. `lower <= 0` on a log odds-ratio axis), leaving the study label orphaned with no marker, whisker, or CI text; and fallback `0.5 * (lo + hi)` (`line 3135`) uses arithmetic midpoint even on log scales.
  2. In `xy::resample_curve_for_x_segment` (`xy.rs:2714-2723`), clamping the curve's transformed domain to `seg_x` evaluates `scale_transform(seg_x.scale, first_x)` and `scale_transform(seg_x.scale, last_x)`. On `AxisScale::Reciprocal`, $1/x$ flips monotonicity across $0$: when `first_x < 0 < last_x` (say `[-5, 5]`), `gx0 = -0.2 < gx1 = 0.2`, intersecting `[ -0.2, 0.2 ]` in reciprocal space (`|x| >= 5`) — completely outside the curve domain `[-5, 5]` while omitting the curve's valid positive/negative branches inside `seg_x`.
- **Failure scenario:** Turning off the legend on a Forest plot clips all study numeric summaries off the right edge; a `Reciprocal` X-axis fitted curve spanning zero disappears or draws across an invalid disconnected region.
- **Evidence:**
  ```rust
  // crates/chart/src/multivariate_layout.rs:2974, 3016-3022, 3054-3061
          PlotMarginConfig {
              extra_left_pt: crate::clamp_f64_safe(max_lbl_w + 10.0, 32.0, 96.0),
              has_legend: true,
              ...
          let (Some(x_lo_pt), Some(x_est_pt), Some(x_hi_pt)) = (
              x_map.map_value(entry.lower, r.x, r.right()),
              x_map.map_value(entry.estimate, r.x, r.right()),
              x_map.map_value(entry.upper, r.x, r.right()),
          ) else {
              continue;
          };
  ```
  ```rust
  // crates/chart/src/xy.rs:2714-2723
      let (lo_t, hi_t) = match (scale_transform(seg_x.scale, first_x), scale_transform(seg_x.scale, last_x)) {
          (Some(gx0), Some(gx1)) => {
              let g_curve_lo = gx0.min(gx1);
              let g_curve_hi = gx0.max(gx1);
              (g_curve_lo.max(g_seg_min.min(g_seg_max)), g_curve_hi.min(g_seg_min.max(g_seg_max)))
          }
          _ => (g_seg_min.min(g_seg_max), g_seg_min.max(g_seg_max)),
      };
  ```
- **Suggested fix:** Add dedicated `extra_right_pt` in `PlotMarginConfig` for Forest summary columns; clamp/clip Forest CI whiskers at `seg.domain_min` instead of skipping rows when `lower <= 0` on log scales, and use geometric midpoint on log scales. In `resample_curve_for_x_segment`, fall back to full `(g_seg_min, g_seg_max)` for `Reciprocal` whenever `src` spans zero (`first_x <= 0.0 && last_x >= 0.0`).
- **Related:** None.

---

### FA-SEC09-11 (S2) — Widespread `GraphSpec` / `AxisSpec` / `CategoricalTickSource` contract drift across `xy`, `survival_cat`, `column_grouped`, and `multivariate_layout`
- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/chart/src/xy.rs:659-683, 786-793, 808-818, 913-918, 985-992, 1593-1599, 2466-2468`, `crates/chart/src/survival_cat.rs:184-222, 277-315, 912`, `crates/chart/src/column_grouped.rs:725-728, 788, 860-893`, `crates/chart/src/multivariate_layout.rs:189-206`
- **Category:** `contract-drift`
- **Defect:** `compile_graph` routes XY/Column/Grouped via `xy.rs` chrome and Survival/Contingency/PartsOfWhole/Multivariate via `survival_cat.rs` chrome, neither of which uses the fuller `scales::render_axes_and_frame` engine:
  1. `survival_cat::{resolve_axis_domain, nice_axis_ticks, CartesianFrame}` ignores broken-axis `segments[1..]` (`scales.rs` supports 1–3 broken segments, while `survival_cat` builds a single `AxisMapping` from `segments.first()`), ignores `FramePreset::OffsetRangeTufte` / `DualYSecondaryRight` (`lines 301-314`), draws `OriginCrossing` spines without moving X/Y ticks to the zero spines, ignores axis `color`, `show_ticks`, and `gridlines`, silently drops axis warnings in `resolve_axis_domain` / `nice_axis_ticks` (`lines 185, 194`), and flattens `AnnotationShapeKind::RoundedRectangle` to `Path2D::rect` (`line 912`).
  2. `xy.rs` detaches X/Y ticks from `OriginCrossing` spines onto `r.bottom()` / `r.x` (`lines 914, 985-992`), does not trim `OffsetRangeTufte` spines to data min/max (`lines 808-814`), skips `expand_doc_text` in threshold (`line 787`) and legend labels (`line 1594`), and uses `.any` instead of `.all` in `is_out_of_axis_domain` (`line 2467`, dropping points valid in one broken-axis segment if another segment uses a log/probit scale).
  3. `column_grouped.rs:870-882` implements `GroupedArrangement::Separated` by emitting `nrows * ncols` combined `"{col_title}:{row_label}"` ticks unconditionally, ignoring `CategoricalTickSource` (`ColumnTitles` default vs `RowTitles` vs `RowNumbers`), while `Interleaved`/`Stacked`/`Superimposed` (`lines 860, 887`) and `Nested` (`line 788`) ignore `CategoricalTickSource::RowNumbers` (only `Column` `line 726` and `Contingency` `line 1982` honor `CategoricalTickSource`).
- **Failure scenario:** A `GroupedArrangement::Separated` plot defaults to showing `"{col}:{row}"` ticks for every bar instead of Column titles; broken Y axes on Survival/Residual/Volcano/Forest plots silently collapse to segment 0; threshold/legend labels with `{{info:...}}` or `{{result:...}}` render raw tokens.
- **Evidence:**
  ```rust
  // crates/chart/src/survival_cat.rs:205-206 & 301-302
      let seg_spec = axis.segments.first();
      let scale = seg_spec.map_or(AxisScale::Linear, |s| s.scale);
  ...
          FramePreset::OpenL | FramePreset::OffsetRangeTufte | FramePreset::DualYSecondaryRight => {
  ```
  ```rust
  // crates/chart/src/xy.rs:2466-2468 & 914
  fn is_out_of_axis_domain(axis: &SegmentedAxis, val: f64) -> bool {
      axis.segments.iter().any(|s| scale_transform(s.scale, val).is_none())
  }
  ...
                        path: Path2D::line(PointPt::new(x_pt, r.bottom() + o0), PointPt::new(x_pt, r.bottom() + o1)),
  ```
  ```rust
  // crates/chart/src/column_grouped.rs:870-882
          GroupedArrangement::Separated => {
              let group_stride = (nrows as f64) + 0.9;
              for c in 0..ncols {
                  for r in 0..nrows {
                      let slot_x = (c as f64) * group_stride + (r as f64) + 0.5;
                      let row_label = row_display_label(&table.row_titles, r);
                      let col_title = table.data_sets.get(c).map_or("", |d| d.title.as_str());
                      tick_positions.push((slot_x, format!("{col_title}:{row_label}")));
  ```
- **Suggested fix:** Consolidate axis/frame/annotation rendering onto `scales.rs` / `xy.rs` with full broken-axis, `FramePreset`, `show_ticks`/`gridlines`, and `expand_doc_text` parity; honor `CategoricalTickSource` across `Separated`, `Interleaved`, `Stacked`, `Superimposed`, and `Nested` extractors; change `is_out_of_axis_domain` to `.all(...)`.
- **Related:** `FA-SEC09-06`, `FA-SEC09-13`.

---

### FA-SEC09-12 (S3) — Resource-budget bypasses on graph annotations/layouts, non-finite annotation coords, and XY overlay segment cap drift
- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/chart/src/xy.rs:1103-1115, 1149, 1179, 1391-1430, 1555-1559, 2685`, `crates/chart/src/survival_cat.rs:758-760, 881-889, 937, 967`, `crates/chart/src/scales.rs:3568`, `crates/chart/src/multivariate_layout.rs:4019`
- **Category:** `resources` / `robustness`
- **Defect:** Five resource-control and robustness gaps span graph/layout compilation:
  1. `GraphAnnotation::EmbeddedImage` passes `pixel_budget = None` across `xy.rs:1179`, `survival_cat.rs:967`, and `scales.rs:3568`, bypassing cumulative RGBA memory budgeting; additionally `multivariate_layout::transform_primitive` (`line 4019`) executes `img.rgba.clone()` per placed panel without counting duplicated allocations against `layout_image_pixels`.
  2. `GraphAnnotation::EmbeddedResultTable` passes `extra_table_work = None` (`xy.rs:1149`, `survival_cat.rs:937`), bypassing `MAX_LAYOUT_PRIMITIVES`.
  3. For `LayoutItem::LiveTable`, `render_styled_embedded_table` (`xy.rs:1416-1430`) computes `est_cells` using unfiltered `rows.len()` *before* viewport clamping (`max_data_rows`), causing truncated small tables on large result sheets to falsely abort with `layout_primitive_budget_exceeded`; and when `extract_sheet_grid` returns `None` because a `ResultSheet` has 0 tabs/scalars, `lines 1392-1394` suppress the missing placeholder border because `doc.result_sheet(spec.sheet_id).is_some()`.
  4. `legend.position_pt` (`xy.rs:1555`, `survival_cat.rs:758`) and `GraphAnnotation::LineOrArrow` (`xy.rs:1103-1107`, `survival_cat.rs:881-885`) do not guard against `NaN`/`Inf` coordinates, emitting non-finite `Path2D` primitives that fail `preflight_check_scene`.
  5. `xy.rs:2685` clamps regression `sample_segments` to `.clamp(256, 512)` instead of `256..=1024` as specified in `docs/implementation/graphing-and-layouts.md` and `doc`.
- **Failure scenario:** A layout embedding many large images or big `LiveTable`s either exceeds memory budgets via repeated `img.rgba.clone()` or falsely aborts layout compilation; non-finite annotation/legend coordinates cause `preflight_check_scene` export rejection.
- **Evidence:**
  ```rust
  // crates/chart/src/xy.rs:1149, 1179 & 2685
                      missing_warning_code: "missing_embedded_result_table",
                  },
                  None,
              );
  ...
                  None,
              );
  ...
      let total_samples = usize::from(overlay.spec.sample_segments.clamp(256, 512));
  ```
  ```rust
  // crates/chart/src/xy.rs:1416-1430
      let est_cells = (rows.len().saturating_add(usize::from(show_hdr_row))).saturating_mul(total_vis_cols);
      if let Some(extra_work) = extra_table_work.as_deref() {
          crate::multivariate_layout::check_layout_primitive_budget(scene.primitives.len().saturating_add(*extra_work), est_cells, "layout table")?;
      }
  ...
      let max_data_rows = ((bounds.height / row_h).floor() as usize).saturating_sub(usize::from(show_hdr_row)).min(rows.len());
  ```
  ```rust
  // crates/chart/src/multivariate_layout.rs:4019
              rgba: img.rgba.clone(),
  ```
- **Suggested fix:** Enforce a shared graph-level `pixel_budget` and track placed-panel image clones; compute `LiveTable` `est_cells` using `max_data_rows`; filter non-finite `legend.position_pt` and `LineOrArrow` endpoints with warnings; raise `sample_segments` clamp to `256..=1024`.
- **Related:** None.

---

### FA-SEC09-13 (S4) — Unused `curvebook-transform` dependency and three redundant chrome/annotation implementations
- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/chart/Cargo.toml:21` and `crates/chart/src/{scales.rs, xy.rs, survival_cat.rs}`
- **Category:** `cleanup` / `architecture`
- **Defect:** Architecture baseline checks passed (`#![forbid(unsafe_code)]` at `crates/chart/src/lib.rs:6`, `[lints] workspace = true` at `crates/chart/Cargo.toml:10-11`, no `chrono`, `wasm32`-clean dependency tree). However, two maintenance issues remain:
  1. `crates/chart/Cargo.toml:21` declares `curvebook-transform = { workspace = true }`, which has zero references anywhere in `crates/chart/src/`.
  2. Axis framing, tick formatting, legends, and annotation rendering are duplicated across three separate engines (`scales.rs`, `xy.rs`, and `survival_cat.rs`), directly causing feature and behavior drift (`FA-SEC09-06`, `FA-SEC09-11`).
- **Failure scenario:** Unused workspace dependency increases incremental compile/link graph; parallel chrome/annotation codepaths cause future `GraphSpec` features to diverge across graph types.
- **Evidence:**
  ```toml
  # crates/chart/Cargo.toml:10-22
  [lints]
  workspace = true

  [dependencies]
  image = { workspace = true }
  curvebook-doc = { workspace = true }
  curvebook-fit = { workspace = true }
  curvebook-math = { workspace = true }
  curvebook-palette = { workspace = true }
  curvebook-stats = { workspace = true }
  curvebook-text = { workspace = true }
  curvebook-transform = { workspace = true }
  serde = { workspace = true }
  ```
- **Suggested fix:** Remove `curvebook-transform = { workspace = true }` from `crates/chart/Cargo.toml` and converge graph chrome, legend, and annotation rendering behind a single shared module.
- **Related:** `FA-SEC09-06`, `FA-SEC09-11`.
