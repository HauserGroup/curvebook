//! Fluent [`ResultTab`] builders and canonical [`ResultCell`] constructors shared across
//! `curvebook-stats`, `curvebook-fit`, and `curvebook-transform`.
//!
//! Eliminates parallel `row_labels: Vec<String>` + `rows: Vec<Vec<ResultCell>>` boilerplate while
//! preserving byte-identical [`ResultTab`] layouts, headers, row labels, cells, and footnotes.

use crate::analysis::PValueFormat;
use crate::result::{CiBound, ResultCell, ResultTab, ResultTabKind};

/// Build a numeric [`ResultCell`], yielding [`ResultCell::Empty`] when `value` is non-finite.
#[must_use]
pub fn num_cell(value: f64) -> ResultCell {
    if value.is_finite() { ResultCell::Number { value, prefix: None, units: None } } else { ResultCell::Empty }
}

/// Build an optional numeric [`ResultCell`], yielding [`ResultCell::Empty`] for `None` or non-finite values.
#[must_use]
pub fn opt_num_cell(value: Option<f64>) -> ResultCell {
    match value {
        Some(v) if v.is_finite() => ResultCell::Number { value: v, prefix: None, units: None },
        _ => ResultCell::Empty,
    }
}

/// Build a [`ResultCell::ConfidenceInterval`], mapping non-finite bounds to [`CiBound::Open`].
#[must_use]
pub fn ci_cell(lower: f64, upper: f64) -> ResultCell {
    let lo = if lower.is_finite() { CiBound::Finite(lower) } else { CiBound::Open };
    let hi = if upper.is_finite() { CiBound::Finite(upper) } else { CiBound::Open };
    ResultCell::ConfidenceInterval { lower: lo, upper: hi }
}

/// Build a [`ResultCell::ConfidenceInterval`] when neither bound is `NaN`, or [`ResultCell::Empty`]
/// if either bound is `NaN`. Infinite bounds still render as [`CiBound::Open`].
#[must_use]
pub fn finite_ci_cell(lower: f64, upper: f64) -> ResultCell {
    if lower.is_nan() || upper.is_nan() { ResultCell::Empty } else { ci_cell(lower, upper) }
}

/// Build a [`ResultCell::ConfidenceInterval`] from `Option<(f64, f64)>`.
#[must_use]
pub fn opt_ci_cell(bounds: Option<(f64, f64)>) -> ResultCell {
    match bounds {
        Some((lo, hi)) => ci_cell(lo, hi),
        None => ResultCell::Empty,
    }
}

/// Build a [`ResultCell::PValue`] storing unrounded `raw` probability and formatted strings.
#[must_use]
pub fn p_cell(raw_p: f64, format: PValueFormat) -> ResultCell {
    if !raw_p.is_finite() {
        return ResultCell::Empty;
    }
    let clamped = raw_p.clamp(0.0, 1.0);
    ResultCell::PValue { raw: clamped, formatted: format.format_p(clamped), stars: format.stars(clamped).to_owned() }
}

/// Build a [`ResultCell::PValue`] storing unrounded `raw` probability, formatted strings, and
/// significance stars evaluated against the analysis threshold `alpha` (`p <= alpha`).
#[must_use]
pub fn p_cell_at_alpha(raw_p: f64, format: PValueFormat, alpha: f64) -> ResultCell {
    if !raw_p.is_finite() {
        return ResultCell::Empty;
    }
    let clamped = raw_p.clamp(0.0, 1.0);
    let stars = if matches!(format, PValueFormat::FourDecimal | PValueFormat::Custom { .. }) {
        curvebook_math::pvalue::p_value_stars_at_alpha(clamped, alpha)
    } else {
        format.stars_at_alpha(clamped, alpha)
    };
    ResultCell::PValue { raw: clamped, formatted: format.format_p(clamped), stars: stars.to_owned() }
}

/// Build a [`ResultCell::Text`] from any string-like value.
#[must_use]
pub fn text_cell(text: impl Into<String>) -> ResultCell {
    ResultCell::Text(text.into())
}

/// Build a [`ResultCell::Boolean`].
#[must_use]
pub const fn bool_cell(value: bool) -> ResultCell {
    ResultCell::Boolean(value)
}

/// Fluent multi-column [`ResultTab`] builder that keeps `row_labels` and `rows` synchronized.
#[derive(Debug, Clone)]
pub struct TabBuilder {
    tab: ResultTab,
}

impl TabBuilder {
    /// Start a new [`ResultTab`] with the given `id`, `title`, and semantic [`ResultTabKind`].
    #[must_use]
    pub fn new(id: impl Into<String>, title: impl Into<String>, kind: ResultTabKind) -> Self {
        Self {
            tab: ResultTab {
                id: id.into(),
                title: title.into(),
                kind,
                column_headers: Vec::new(),
                row_labels: Vec::new(),
                rows: Vec::new(),
                footnotes: Vec::new(),
            },
        }
    }

    /// Set the column headers from an iterator of string-like titles.
    #[must_use]
    pub fn headers<I, S>(mut self, headers: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.tab.column_headers = headers.into_iter().map(Into::into).collect();
        self
    }

    /// Append a labeled row of cells (consuming builder form).
    #[must_use]
    pub fn row<I>(mut self, label: impl Into<String>, cells: I) -> Self
    where
        I: IntoIterator<Item = ResultCell>,
    {
        self.push_row(label, cells);
        self
    }

    /// Append a labeled row of cells in place (for conditional or loop-driven construction).
    pub fn push_row<I>(&mut self, label: impl Into<String>, cells: I) -> &mut Self
    where
        I: IntoIterator<Item = ResultCell>,
    {
        self.tab.row_labels.push(label.into());
        self.tab.rows.push(cells.into_iter().collect());
        self
    }

    /// Append multiple `(label, cells)` rows at once.
    #[must_use]
    pub fn rows<I, L, R>(mut self, rows: I) -> Self
    where
        I: IntoIterator<Item = (L, R)>,
        L: Into<String>,
        R: IntoIterator<Item = ResultCell>,
    {
        for (label, cells) in rows {
            self.push_row(label, cells);
        }
        self
    }

    /// Append a single footnote (consuming builder form).
    #[must_use]
    pub fn footnote(mut self, note: impl Into<String>) -> Self {
        self.tab.footnotes.push(note.into());
        self
    }

    /// Append a single footnote in place.
    pub fn push_footnote(&mut self, note: impl Into<String>) -> &mut Self {
        self.tab.footnotes.push(note.into());
        self
    }

    /// Append multiple footnotes at once.
    #[must_use]
    pub fn footnotes<I, S>(mut self, notes: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.tab.footnotes.extend(notes.into_iter().map(Into::into));
        self
    }

    /// Finish construction and return the built [`ResultTab`].
    #[must_use]
    pub fn build(self) -> ResultTab {
        self.tab
    }
}

/// Fluent single-column key-value [`ResultTab`] builder pairing each row label with its value cell.
#[derive(Debug, Clone)]
pub struct KvTabBuilder {
    tab: ResultTab,
}

impl KvTabBuilder {
    /// Start a single-column key-value [`ResultTab`] with header `value_header`.
    #[must_use]
    pub fn new(id: impl Into<String>, title: impl Into<String>, kind: ResultTabKind, value_header: impl Into<String>) -> Self {
        Self {
            tab: ResultTab {
                id: id.into(),
                title: title.into(),
                kind,
                column_headers: vec![value_header.into()],
                row_labels: Vec::new(),
                rows: Vec::new(),
                footnotes: Vec::new(),
            },
        }
    }

    /// Append a labeled raw [`ResultCell`] (consuming builder form).
    #[must_use]
    pub fn cell(mut self, label: impl Into<String>, cell: ResultCell) -> Self {
        self.push_cell(label, cell);
        self
    }

    /// Append a labeled raw [`ResultCell`] in place.
    pub fn push_cell(&mut self, label: impl Into<String>, cell: ResultCell) -> &mut Self {
        self.tab.row_labels.push(label.into());
        self.tab.rows.push(vec![cell]);
        self
    }

    /// Append a labeled numeric cell via [`num_cell`].
    #[must_use]
    pub fn num(mut self, label: impl Into<String>, value: f64) -> Self {
        self.push_num(label, value);
        self
    }

    /// Append a labeled numeric cell in place via [`num_cell`].
    pub fn push_num(&mut self, label: impl Into<String>, value: f64) -> &mut Self {
        self.push_cell(label, num_cell(value))
    }

    /// Append a labeled optional numeric cell via [`opt_num_cell`].
    #[must_use]
    pub fn opt_num(mut self, label: impl Into<String>, value: Option<f64>) -> Self {
        self.push_opt_num(label, value);
        self
    }

    /// Append a labeled optional numeric cell in place via [`opt_num_cell`].
    pub fn push_opt_num(&mut self, label: impl Into<String>, value: Option<f64>) -> &mut Self {
        self.push_cell(label, opt_num_cell(value))
    }

    /// Append a labeled *p*-value cell via [`p_cell_at_alpha`].
    #[must_use]
    pub fn p(mut self, label: impl Into<String>, raw_p: f64, format: PValueFormat, alpha: f64) -> Self {
        self.push_p(label, raw_p, format, alpha);
        self
    }

    /// Append a labeled *p*-value cell in place via [`p_cell_at_alpha`].
    pub fn push_p(&mut self, label: impl Into<String>, raw_p: f64, format: PValueFormat, alpha: f64) -> &mut Self {
        self.push_cell(label, p_cell_at_alpha(raw_p, format, alpha))
    }

    /// Append a labeled confidence-interval cell via [`ci_cell`].
    #[must_use]
    pub fn ci(mut self, label: impl Into<String>, lower: f64, upper: f64) -> Self {
        self.push_ci(label, lower, upper);
        self
    }

    /// Append a labeled confidence-interval cell in place via [`ci_cell`].
    pub fn push_ci(&mut self, label: impl Into<String>, lower: f64, upper: f64) -> &mut Self {
        self.push_cell(label, ci_cell(lower, upper))
    }

    /// Append a labeled NaN-guarded confidence-interval cell via [`finite_ci_cell`].
    #[must_use]
    pub fn finite_ci(mut self, label: impl Into<String>, lower: f64, upper: f64) -> Self {
        self.push_finite_ci(label, lower, upper);
        self
    }

    /// Append a labeled NaN-guarded confidence-interval cell in place via [`finite_ci_cell`].
    pub fn push_finite_ci(&mut self, label: impl Into<String>, lower: f64, upper: f64) -> &mut Self {
        self.push_cell(label, finite_ci_cell(lower, upper))
    }

    /// Append a labeled optional confidence-interval cell via [`opt_ci_cell`].
    #[must_use]
    pub fn opt_ci(mut self, label: impl Into<String>, bounds: Option<(f64, f64)>) -> Self {
        self.push_opt_ci(label, bounds);
        self
    }

    /// Append a labeled optional confidence-interval cell in place via [`opt_ci_cell`].
    pub fn push_opt_ci(&mut self, label: impl Into<String>, bounds: Option<(f64, f64)>) -> &mut Self {
        self.push_cell(label, opt_ci_cell(bounds))
    }

    /// Append a labeled text cell via [`text_cell`].
    #[must_use]
    pub fn text(mut self, label: impl Into<String>, value: impl Into<String>) -> Self {
        self.push_text(label, value);
        self
    }

    /// Append a labeled text cell in place via [`text_cell`].
    pub fn push_text(&mut self, label: impl Into<String>, value: impl Into<String>) -> &mut Self {
        self.push_cell(label, text_cell(value))
    }

    /// Append a labeled boolean cell via [`bool_cell`].
    #[must_use]
    pub fn boolean(mut self, label: impl Into<String>, value: bool) -> Self {
        self.push_boolean(label, value);
        self
    }

    /// Append a labeled boolean cell in place via [`bool_cell`].
    pub fn push_boolean(&mut self, label: impl Into<String>, value: bool) -> &mut Self {
        self.push_cell(label, bool_cell(value))
    }

    /// Append a footnote (consuming builder form).
    #[must_use]
    pub fn footnote(mut self, note: impl Into<String>) -> Self {
        self.tab.footnotes.push(note.into());
        self
    }

    /// Append a footnote in place.
    pub fn push_footnote(&mut self, note: impl Into<String>) -> &mut Self {
        self.tab.footnotes.push(note.into());
        self
    }

    /// Finish construction and return the built [`ResultTab`].
    #[must_use]
    pub fn build(self) -> ResultTab {
        self.tab
    }
}

#[cfg(test)]
mod tests {
    use super::{
        KvTabBuilder, TabBuilder, bool_cell, ci_cell, finite_ci_cell, num_cell, opt_ci_cell, opt_num_cell, p_cell, p_cell_at_alpha, text_cell,
    };
    use crate::{CiBound, PValueFormat, ResultCell, ResultTabKind};

    #[test]
    fn cell_constructors_handle_finite_nan_inf_and_pvalues() {
        assert_eq!(num_cell(2.5), ResultCell::Number { value: 2.5, prefix: None, units: None });
        assert_eq!(num_cell(f64::NAN), ResultCell::Empty);
        assert_eq!(num_cell(f64::INFINITY), ResultCell::Empty);
        assert_eq!(opt_num_cell(Some(1.0)), num_cell(1.0));
        assert_eq!(opt_num_cell(Some(f64::NAN)), ResultCell::Empty);
        assert_eq!(opt_num_cell(None), ResultCell::Empty);

        assert_eq!(ci_cell(1.0, 2.0), ResultCell::ConfidenceInterval { lower: CiBound::Finite(1.0), upper: CiBound::Finite(2.0) });
        assert_eq!(ci_cell(f64::NEG_INFINITY, 2.0), ResultCell::ConfidenceInterval { lower: CiBound::Open, upper: CiBound::Finite(2.0) });
        assert_eq!(finite_ci_cell(f64::NAN, 2.0), ResultCell::Empty);
        assert_eq!(finite_ci_cell(f64::NEG_INFINITY, 2.0), ci_cell(f64::NEG_INFINITY, 2.0));
        assert_eq!(opt_ci_cell(None), ResultCell::Empty);

        assert_eq!(p_cell(f64::NAN, PValueFormat::FourDecimal), ResultCell::Empty);
        assert_eq!(
            p_cell_at_alpha(0.03, PValueFormat::FourDecimal, 0.01),
            ResultCell::PValue { raw: 0.03, formatted: String::from("0.0300"), stars: String::from("ns") }
        );
        assert_eq!(text_cell("ok"), ResultCell::Text(String::from("ok")));
        assert_eq!(bool_cell(true), ResultCell::Boolean(true));
    }

    #[test]
    fn tab_and_kv_builders_produce_exact_result_tabs() {
        let multi = TabBuilder::new("t1", "Table 1", ResultTabKind::AnovaTable)
            .headers(["SS", "df", "P"])
            .row("Treatment", [num_cell(12.0), num_cell(2.0), p_cell(0.004, PValueFormat::FourDecimal)])
            .footnote("Note A")
            .build();
        assert_eq!(multi.id, "t1");
        assert_eq!(multi.column_headers, vec!["SS", "df", "P"]);
        assert_eq!(multi.row_labels, vec!["Treatment"]);
        assert_eq!(multi.rows.len(), 1);
        assert_eq!(multi.rows[0].len(), 3);
        assert_eq!(multi.footnotes, vec!["Note A"]);

        let mut kv = KvTabBuilder::new("kv1", "Summary", ResultTabKind::MainSummary, "Value")
            .num("Mean", 3.5)
            .ci("95% CI", 2.0, 5.0)
            .text("Method", "Student t");
        kv.push_p("P value", 0.02, PValueFormat::FourDecimal, 0.05);
        kv.push_boolean("Significant", true);
        let tab = kv.footnote("Two-tailed").build();
        assert_eq!(tab.column_headers, vec!["Value"]);
        assert_eq!(tab.row_labels, vec!["Mean", "95% CI", "Method", "P value", "Significant"]);
        assert_eq!(tab.rows.len(), 5);
        assert_eq!(tab.footnotes, vec!["Two-tailed"]);
    }
}
