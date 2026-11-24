# Verification Verdicts: SEC-09 — Chart Compiler & Scales (`crates/chart`)

- **Section:** `SEC-09` (Chart Compiler & Scales)
- **Source Report:** [`docs/audit/full-2026-11-22/SEC-09.md`](SEC-09.md)
- **Baseline SHA:** `7c85a8e516165ddadb101236a88eec937bf1f095` (`main`)
- **Verifier:** `ver-frontend`
- **Date:** `2026-11-22`

## Summary

| Verdict | Count | Finding IDs |
|---|---:|---|
| `CONFIRMED` | 13 | `FA-SEC09-01`..`FA-SEC09-13` |
| `PLAUSIBLE` | 0 | — |
| `REFUTED` | 0 | — |
| `DUPLICATE` | 0 | — |

| Settled Severity | Count | Finding IDs |
|---|---:|---|
| `S1` | 5 | `FA-SEC09-01`, `FA-SEC09-02`, `FA-SEC09-03`, `FA-SEC09-04`, `FA-SEC09-05` |
| `S2` | 6 | `FA-SEC09-06`, `FA-SEC09-07`, `FA-SEC09-08`, `FA-SEC09-09`, `FA-SEC09-10`, `FA-SEC09-11` |
| `S3` | 1 | `FA-SEC09-12` |
| `S4` | 1 | `FA-SEC09-13` |

## File Coverage Completeness Check

- **Result:** **PASS (9 / 9 source files + `crates/chart/Cargo.toml`, 19,978 LOC — 100% covered)**
- Independently verified all files in `crates/chart/Cargo.toml` (`26` LOC) and `crates/chart/src/` (`column_grouped.rs` `2,723`, `equation_box.rs` `2,595`, `image_decode.rs` `189`, `lib.rs` `207`, `multivariate_layout.rs` `4,335`, `scales.rs` `3,851`, `scene.rs` `482`, `survival_cat.rs` `2,459`, `xy.rs` `3,111`) via directory enumeration and `wc -l`. Every file and exact LOC matches the File Coverage Table in `SEC-09.md`.

---

## Per-Finding Verdicts

### Verdict `FA-SEC09-01` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S1` (infinite loop & OOM abort on malformed TeX script strings)
- **Evidence:**
  - `crates/chart/src/equation_box.rs:681-711`: `parse_math_slice` loops `while pos < input.len()`.
  - `equation_box.rs:758-763`: when `ch == '^' || ch == '_'`, `parse_next_step` does not advance `pos`, calling `attach_scripts_if_any(empty, input, pos, text_mode, depth)` and returning `Some(ParsedStep::Raw(attached))`.
  - `equation_box.rs:1033-1056, 1061-1070, 1110-1138`: in `attach_scripts_if_any`, if `parse_script_arg` returns `None` (true for `"x^"`, `"x_ "`, `"y = ^"`, and `"y = ^{unclosed"` because `trimmed.chars().next()?` or `extract_braced(trimmed)?` returns `None`), it executes `break;` without modifying `*pos`. `parse_math_slice` then pushes `MathNode::Group(Vec::new())` and re-enters `parse_next_step` at the exact same `pos` indefinitely until OOM. (Precision note: `"^}"` actually advances `pos` via `parse_literal_atom` at `1080`, whereas `"x^"`, `"x_ "`, `"y = ^"`, `"y = ^{unclosed"`, and `depth >= MAX_TEX_DEPTH` loop forever.)

### Verdict `FA-SEC09-02` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S1` (silent statistical inaccuracies in `CategoricalSummary`)
- **Evidence:**
  - `crates/chart/src/multivariate_layout.rs:612, 988-991`: `ErrorBarMetric::ConfidenceInterval95` uses hardcoded `1.96 * sd / (n as f64).sqrt()` instead of Student-$t$ critical values (`xy.rs:258` uses `student_t_two_sided_isf(0.05, ...)`), with no `n < 2` check.
  - `multivariate_layout.rs:764-770`: `SummaryMeasure::GeometricMean` silently falls back to the arithmetic mean when any value is `<= 0.0` (contrast `xy.rs:203-210`, which emits `non_positive_geometric_mean` and returns `None`).
  - `multivariate_layout.rs:655-679`: `data_lo`/`data_hi` computes bounds only from raw group observations, ignoring `y_error_bars` bounds.
  - `multivariate_layout.rs:1004-1022`: `emit_error_bar` returns early without drawing if either `y_lo` or `y_hi` fails `map_value`, and draws horizontal end caps without checking `eb.cap_width_pt > 0.0`.
  - `multivariate_layout.rs:744, 779-797`: `BeforeAfterLines`/`ScatterAndLine` connecting lines hardcode `graph.spec.palette.color_at(0)`, and `emit_replicate_dots` neither calls `resolve_overridden_marker_for_cell` nor pushes `scene.plotted_points`.

### Verdict `FA-SEC09-03` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S1` (inverted domains, dropped sub-picomolar log domains, and broken extreme-range mapping)
- **Evidence:**
  - `crates/chart/src/scales.rs:1324-1333`: when `lo <= 0.0` on `Log10`/`Log2`/`Ln` and `raw_lo <= 0.0`, `lo` is set to `0.1`; if `hi` (e.g. `0.05`) is positive and `< 0.1`, `(hi - lo).abs() = 0.05 > 1e-12` passes and returns inverted `(0.1, 0.05)`.
  - `scales.rs:1309, 1330, 1342, 1357`: checks `init_span.abs() <= 1e-12` and `(hi - lo).abs() <= 1e-12` in linear data space on `Log10`/`Log2`/`Ln`/`Logit`/`Probit`/`Reciprocal`, causing `[1e-14, 1e-12]` to fail and fall back to `sample_default_domain` (`(1.0, 100.0)`).
  - `scales.rs:1185-1191`: `if seg_valid.is_empty()` prevents `extend_upper_bound`/`extend_lower_bound` from firing when `seg_valid` is non-empty and single-sided `explicit_min > raw_hi` or `explicit_max < raw_lo`; on `Linear`, `is_degenerate_domain` (`1011-1014`) checks `span.abs() <= MIN_FINITE_AXIS_SPAN`, so negative `dmax - dmin` passes and returns an inverted domain.
  - `scales.rs:1246-1253, 192-195`: `sanitize_linear_range(-1e308, 1e308, ...)` overflows `span = hi - lo` to `+Inf`, triggers `!span.is_finite()`, widens to `(-1.1e308, 1.1e308)`, and causes `denom = t_max - t_min = +Inf` in `AxisMapping::map_value`, returning `None` for all points.

### Verdict `FA-SEC09-04` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S1` (silent wrong Bland-Altman data pairing on XY and MV tables)
- **Evidence:**
  - `crates/doc/src/analysis.rs:415-428`: `BlandAltmanSpec` defines `xy_compare_x_vs_y: Option<bool>` (`Some(false)` or `None` with `>= 2` Y sets compares `Y[column_a]` vs `Y[column_b]`).
  - `crates/chart/src/multivariate_layout.rs:2615-2627`: `TableArchetype::Xy(xy)` ignores `xy_compare_x_vs_y` and `col_b` unconditionally, calling `active_xy_pairs_with_sources(&ds.id, xy, col_a)` (`X` vs `Y[col_a]`).
  - `multivariate_layout.rs:2596-2600`: `TableArchetype::MultipleVariables` overrides `col_a`/`col_b` (even when resolved from a `BlandAltman` `ResultSheet` at `2638-2639`) with `graph.spec.multivariate`'s `x_column`/`y_column`.
  - `multivariate_layout.rs:2517-2519`: reference line labels hardcode `"+1.96 SD: {loa_hi:.2}"` and `"-1.96 SD: {loa_lo:.2}"`.

### Verdict `FA-SEC09-05` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S1` (silent averaging of summary stats, SDs, and sample sizes as raw replicates)
- **Evidence:**
  - `crates/doc/src/archetypes.rs:831-836`: `GroupedTable::active_replicates` collects `Cell::active_number` across every subcolumn indiscriminately without checking `self.subcolumn_mode.is_pre_summarized()`.
  - `crates/chart/src/multivariate_layout.rs:1742-1750`: Grouped Heatmap extraction calls `g.active_replicates(r, c)` and averages all returned numbers (`Mean`, `SD`, `N`).
  - `multivariate_layout.rs:1150-1164, 3350, 3675`: `collect_ds_active_values_with_sources` iterates `for (subcol, cell) in row_cells.iter().enumerate()` without checking `col.subcolumn_mode.is_pre_summarized()`, feeding `SD`/`SEM`/`N` into Residual and Normal Q-Q fallback plots.

### Verdict `FA-SEC09-06` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (stale failed results rendered silently and wrong-dataset scalar fallback)
- **Evidence:**
  - `crates/chart/src/survival_cat.rs:141-166`: `resolve_result_scalar` does not check `res.status`, and line `159` falls back to any dataset's scalar matching `key` when `data_set_index: Some(_)` misses.
  - `survival_cat.rs:1016-1029` (`extract_sheet_grid`) and `1241-1266` (`extract_km_cohorts`), plus `crates/chart/src/multivariate_layout.rs:1029-1043` (`find_matching_result`, `matching_results`), never check `ResultStatus::Failed` (contrast `equation_box.rs:182-188`, `xy.rs:116`, and `column_grouped.rs:2156-2162`).

### Verdict `FA-SEC09-07` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (silently omitted series geometry in horizontal and Grouped before-after modes)
- **Evidence:**
  - `crates/chart/src/column_grouped.rs:238-247`: `compile_horizontal_column_or_grouped` passes `y2: None` to `render_cartesian_chrome_full` and omits CLD/comparison bracket calls.
  - `column_grouped.rs:271-275`: `render_horizontal_item` discards `BeforeAfterLines`, `BoxAndWhisker`, and `Violin` via `=> {}` with zero warning.
  - `column_grouped.rs:1114-1116, 1199`: `render_before_after_connectors` early-returns unless `TableArchetype::Column`, and `render_single_item` returns `None` for `BeforeAfterLines`, leaving `GroupedTable` before-after plots blank.

### Verdict `FA-SEC09-08` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (collapsed pre-summarized glyphs, unsorted hinges, silent log clamp, reversed KDE collapse)
- **Evidence:**
  - `crates/chart/src/column_grouped.rs:771, 964` and `crates/chart/src/xy.rs:241-244`: `extract_column_presummarized` and `push_grouped_cell` pass `y_error_bars.as_ref()` to `summarize_subcells`; when `y_error_bars` is `None` (typical for `BoxAndWhisker`/`Violin`/`FloatingBar`), `summarize_pre_summarized` returns `(center, center, center)`.
  - `column_grouped.rs:1420-1424, 1571-1575`: build `vec![lower, center, upper]` without sorting before `quartiles_hinge(&sorted)`.
  - `column_grouped.rs:1433-1437, 1749-1750`: discard `.2` (`clipped`) returned by `y_axis.map_clamped_to_nearest(...)`.
  - `column_grouped.rs:1682-1690`: `build_kde_profile` clamps `hi = hi.min(y_axis.segments.last().map_or(hi, |s| s.domain_max))`; on a reversed Y axis (`domain_min > domain_max`), `hi <= lo` triggers `return Vec::new()`, reducing the violin (`1586-1595`) to a horizontal median line.

### Verdict `FA-SEC09-09` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (spurious KM censoring ticks, non-finite propagation, dropped contingency bottom slice)
- **Evidence:**
  - `crates/chart/src/survival_cat.rs:1551-1557`: `else if ev_now == 0 && r_now > 0` pushes censoring tick at every zero-event NAR row even when `prev_risk` is `None` ($t=0$) or `prev_risk == Some(r_now)`.
  - `survival_cat.rs:1480-1496, 1151-1155`: `build_km_from_result_sheet` does not filter non-finite `p.x`/`p.y`/`ci_lower`/`ci_upper`, and `max_time` uses `.fold(1.0_f64, f64::max)`.
  - `survival_cat.rs:1903-1910`: stacked contingency starts at `acc = 0.0`; on a log Y axis `y_map.map_value(0.0, ...)` returns `None`, hitting `else { acc = top_val; continue; }` and dropping slice `0` silently.

### Verdict `FA-SEC09-10` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (Forest text clipping & row loss; reciprocal XY curve range bug)
- **Evidence:**
  - `crates/chart/src/multivariate_layout.rs:2974` & `crates/chart/src/survival_cat.rs:247-251`: `compile_forest_plot` relies on `has_legend: true` for right margin, which `update_plot_rect_for_labels` reduces to `18.0 pt` whenever `!spec.legend.visible` or `spec.legend.position_pt.is_some()`.
  - `multivariate_layout.rs:3016-3022, 3054-3061`: `continue` skips drawing marker, whisker, AND numeric CI text if any of `(lower, estimate, upper)` fails `map_value`; line `3135` falls back to `0.5 * (lo + hi)` on log axes.
  - `crates/chart/src/xy.rs:2714-2723`: for `AxisScale::Reciprocal`, `first_x < 0 < last_x` (e.g. `[-5, 5]`) yields `gx0 = -0.2, gx1 = 0.2`, intersecting `[-0.2, 0.2]` in reciprocal space (`|x| >= 5`) rather than the curve's interior inside positive or negative `seg_x` domains.

### Verdict `FA-SEC09-11` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (multi-path `GraphSpec`/`AxisSpec`/`CategoricalTickSource` contract drift)
- **Evidence:**
  - `crates/chart/src/survival_cat.rs:184-222, 277-315, 412-548, 912`: ignores `segments[1..]`, ignores `FramePreset::OffsetRangeTufte` / `DualYSecondaryRight` (`301`), never repositions ticks on `OriginCrossing` spines, omits minor ticks and `y_axis.tick_angle_deg`, drops axis warnings (`185, 194`), and flattens `RoundedRectangle` to `Path2D::rect` (`912`). (Precision note: `AxisSpec` in `crates/doc/src/graph.rs:396-422` has no `color`, `show_ticks`, or `gridlines` fields — those live on `scales::AxisRenderOptions` — while `survival_cat` indeed ignores `AxisSpec` minor ticks and `y_axis.tick_angle_deg`.)
  - `crates/chart/src/xy.rs:787, 808-814, 914, 985-992, 1594, 2466-2468`: emits ticks fixed at `r.bottom()`/`r.x` even on `OriginCrossing`, does not trim `OffsetRangeTufte` spines (`scales.rs:2904-2906`), omits `expand_doc_text` on thresholds (`787`) and legends (`1594`), and `is_out_of_axis_domain` uses `.any(...)` instead of `.all(...)` across multi-segment axes (`2467`).
  - `crates/chart/src/column_grouped.rs:788, 860, 870-888`: `Separated` unconditionally emits `"{col_title}:{row_label}"` ignoring `CategoricalTickSource`; `Interleaved`, `Stacked`, `Superimposed`, and `Nested` ignore `CategoricalTickSource::RowNumbers`.

### Verdict `FA-SEC09-12` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` (resource-budget bypasses, pre-clamp rejection, non-finite annotation coords, cap drift)
- **Evidence:**
  - `crates/chart/src/xy.rs:1149, 1179`, `survival_cat.rs:937, 967`, `scales.rs:3568` pass `None` for `extra_table_work` and `pixel_budget`; `multivariate_layout.rs:4019` calls `img.rgba.clone()` per panel without checking cumulative `layout_image_pixels`.
  - `xy.rs:1392-1394`: skips placeholder border when `doc.result_sheet(spec.sheet_id).is_some()` and `extract_sheet_grid` returns `None`.
  - `xy.rs:1416-1430`: checks `check_layout_primitive_budget` against unfiltered `rows.len()` before `max_data_rows` truncation (`1430`).
  - `xy.rs:1103-1107, 1555-1559` and `survival_cat.rs:758-760, 881-885`: no `is_finite()` check on `LineOrArrow` endpoints or `legend.position_pt`.
  - `xy.rs:2685`: `.clamp(256, 512)` contradicts `[256, 1024]` in `crates/doc/src/graph.rs:776-777` and `docs/implementation/graphing-and-layouts.md:28`.

### Verdict `FA-SEC09-13` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S4` (unused workspace dependency and duplicated chrome/annotation engines)
- **Evidence:**
  - `crates/chart/Cargo.toml:21`: declares `curvebook-transform = { workspace = true }` with zero occurrences across `crates/chart/src/`.
  - `crates/chart/src/scales.rs`, `xy.rs`, and `survival_cat.rs` maintain three parallel axis/frame/annotation implementations.
