#![forbid(unsafe_code)]

//! Clean-room synthetic datasets shared across `WF-01`–`WF-65` human-path GUI and reference-Session
//! workflow tests (`R10.26`, `R10.28`).
//!
//! All tables, cohort identifiers, labels, and numerical observations in this module are original
//! synthetic fixtures designed for statistical conditioning (non-collinear covariates, interior
//! separation overlap for logistic MLE, identifiable Hill slopes, exact contingency integer counts,
//! and multi-level hierarchical variance components).

use curvebook_doc::{
    Cell, ColumnDesignHint, ColumnTable, ContingencyTable, DataSheet, DocError, GroupedTable, MultipleVariablesTable, NestedTable, SubcolumnMode,
    SurvivalTable, TableArchetype, ThreeWayColumnGrouping, VariableRole, XyTable,
};

/// Single-value entry in an [`MvFixture`] observation row.
#[derive(Debug, Clone, PartialEq)]
pub enum MvCellValue {
    /// Quantitative numeric value.
    Number(f64),
    /// Categorical or identifier string token.
    Text(&'static str),
}

impl MvCellValue {
    /// Render this cell as a string suitable for GUI cell entry or TSV clipboard paste.
    #[must_use]
    pub fn as_token(&self) -> String {
        match self {
            Self::Number(v) => format!("{v}"),
            Self::Text(s) => (*s).to_owned(),
        }
    }

    fn to_doc_cell(&self) -> Cell {
        match self {
            Self::Number(v) => Cell::number(*v),
            Self::Text(s) => Cell::text(*s),
        }
    }
}

/// Column metadata for an [`MvFixture`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MvColumnSpec {
    /// Column title.
    pub title: &'static str,
    /// Explicit semantic variable role.
    pub role: VariableRole,
    /// Lowercase role slug accepted by `WorkflowRun::set_mv_role` (`"continuous"`, `"categorical"`, `"label"`).
    pub role_slug: &'static str,
}

/// Clean-room Multiple Variables fixture (Cox PH, GLM, qPCR, Clustering).
#[derive(Debug, Clone, PartialEq)]
pub struct MvFixture {
    /// Sheet display title.
    pub title: &'static str,
    /// Ordered variable definitions.
    pub columns: Vec<MvColumnSpec>,
    /// Observation rows (`rows[r].len() == columns.len()`).
    pub rows: Vec<Vec<MvCellValue>>,
}

impl MvFixture {
    /// Render headers and rows as tab-separated values for GUI clipboard paste tests.
    #[must_use]
    pub fn to_tsv(&self) -> String {
        let mut lines = Vec::with_capacity(self.rows.len() + 1);
        let hdr: Vec<&str> = self.columns.iter().map(|c| c.title).collect();
        lines.push(hdr.join("\t"));
        for row in &self.rows {
            let cells: Vec<String> = row.iter().map(MvCellValue::as_token).collect();
            lines.push(cells.join("\t"));
        }
        lines.join("\n")
    }

    /// Build a [`DataSheet`] directly for headless `Session` oracle verification (`verify_reference`).
    pub fn to_data_sheet(&self, sheet_id: &str) -> Result<DataSheet, DocError> {
        let mut mv = MultipleVariablesTable::new(self.columns.len(), self.rows.len())?;
        for (c, spec) in self.columns.iter().enumerate() {
            mv.variables[c].title = spec.title.to_owned();
            mv.variables[c].role = spec.role;
            mv.variables[c].role_inferred = false;
        }
        for (r, row) in self.rows.iter().enumerate() {
            for (c, val) in row.iter().enumerate() {
                mv.variables[c].cells[r] = val.to_doc_cell();
            }
        }
        DataSheet::new(sheet_id, self.title, TableArchetype::MultipleVariables(mv))
    }
}

/// Clean-room Kaplan–Meier survival cohort table fixture.
#[derive(Debug, Clone, PartialEq)]
pub struct SurvivalFixture {
    /// Sheet display title.
    pub title: &'static str,
    /// Time column title.
    pub time_title: &'static str,
    /// Cohort arm titles (`len() >= 2`).
    pub cohort_titles: Vec<&'static str>,
    /// Subject rows: `(elapsed_time, per_cohort_status)` where `Some(1)` = event, `Some(0)` = censored, `None` = empty.
    pub rows: Vec<(f64, Vec<Option<i64>>)>,
}

impl SurvivalFixture {
    /// Render as tab-separated text (`Time`, then one column per cohort arm).
    #[must_use]
    pub fn to_tsv(&self) -> String {
        let mut lines = Vec::with_capacity(self.rows.len() + 1);
        let mut hdr = vec![self.time_title];
        hdr.extend_from_slice(&self.cohort_titles);
        lines.push(hdr.join("\t"));
        for (time, cohorts) in &self.rows {
            let mut cols = Vec::with_capacity(1 + cohorts.len());
            cols.push(format!("{time}"));
            for status in cohorts {
                match status {
                    Some(v) => cols.push(format!("{v}")),
                    None => cols.push(String::new()),
                }
            }
            lines.push(cols.join("\t"));
        }
        lines.join("\n")
    }

    /// Build a [`SurvivalTable`]-backed [`DataSheet`] for headless `Session` oracle verification.
    pub fn to_data_sheet(&self, sheet_id: &str) -> Result<DataSheet, DocError> {
        let mut surv = SurvivalTable::new(self.cohort_titles.len(), self.rows.len())?;
        surv.time_title = self.time_title.to_owned();
        for (c, title) in self.cohort_titles.iter().enumerate() {
            surv.cohorts[c].title = (*title).to_owned();
        }
        for (r, (time, cohorts)) in self.rows.iter().enumerate() {
            surv.times[r] = Cell::number(*time);
            for (c, status) in cohorts.iter().enumerate() {
                surv.cohorts[c].cells[r][0] = match status {
                    Some(code) => Cell::number(*code as f64),
                    None => Cell::empty(),
                };
            }
        }
        DataSheet::new(sheet_id, self.title, TableArchetype::Survival(surv))
    }
}

/// One *X* row across `k` response series with replicate subcolumns.
#[derive(Debug, Clone, PartialEq)]
pub struct XySeriesRow {
    /// Independent *X* coordinate.
    pub x: f64,
    /// `series_replicates[series_idx][subcol_idx]`.
    pub series_replicates: Vec<Vec<f64>>,
}

/// Clean-room *X–Y* dose-response / enzyme kinetics fixture (4PL, 5PL, Michaelis–Menten).
#[derive(Debug, Clone, PartialEq)]
pub struct XyReplicateFixture {
    /// Sheet display title.
    pub title: &'static str,
    /// Independent *X* column title.
    pub x_title: &'static str,
    /// Subcolumn replicate count per series.
    pub replicates: usize,
    /// Response series titles.
    pub series_titles: Vec<&'static str>,
    /// Data rows.
    pub rows: Vec<XySeriesRow>,
}

impl XyReplicateFixture {
    /// Render as tab-separated text (`X` followed by every series replicate column).
    #[must_use]
    pub fn to_tsv(&self) -> String {
        let mut lines = Vec::with_capacity(self.rows.len() + 1);
        let mut hdr = vec![self.x_title.to_owned()];
        for title in &self.series_titles {
            for rep in 1..=self.replicates {
                if self.replicates == 1 {
                    hdr.push((*title).to_owned());
                } else {
                    hdr.push(format!("{title}_R{rep}"));
                }
            }
        }
        lines.push(hdr.join("\t"));
        for row in &self.rows {
            let mut cols = vec![format!("{}", row.x)];
            for series in &row.series_replicates {
                for &val in series {
                    cols.push(format!("{val}"));
                }
            }
            lines.push(cols.join("\t"));
        }
        lines.join("\n")
    }

    /// Build an [`XyTable`]-backed [`DataSheet`] for headless `Session` oracle verification.
    pub fn to_data_sheet(&self, sheet_id: &str) -> Result<DataSheet, DocError> {
        let mode = if self.replicates <= 1 { SubcolumnMode::Single } else { SubcolumnMode::RawReplicates(self.replicates) };
        let mut xy = XyTable::new(mode, self.series_titles.len(), self.rows.len())?;
        xy.x_title = self.x_title.to_owned();
        for (c, title) in self.series_titles.iter().enumerate() {
            xy.data_sets[c].title = (*title).to_owned();
        }
        for (r, row) in self.rows.iter().enumerate() {
            xy.x_cells[r][0] = Cell::number(row.x);
            for (c, series) in row.series_replicates.iter().enumerate() {
                for (s, &val) in series.iter().enumerate() {
                    xy.data_sets[c].cells[r][s] = Cell::number(val);
                }
            }
        }
        DataSheet::new(sheet_id, self.title, TableArchetype::Xy(xy))
    }
}

/// Clean-room contingency count-table fixture (`2x2`, `2x3`, `3x3`).
#[derive(Debug, Clone, PartialEq)]
pub struct ContingencyFixture {
    /// Sheet display title.
    pub title: &'static str,
    /// Row factor title.
    pub row_factor_title: &'static str,
    /// Column factor title.
    pub column_factor_title: &'static str,
    /// Row labels.
    pub row_titles: Vec<&'static str>,
    /// Column category labels.
    pub column_titles: Vec<&'static str>,
    /// Integer cell counts (`counts[row][col]`).
    pub counts: Vec<Vec<u64>>,
}

impl ContingencyFixture {
    /// Render as tab-separated count rows prefixed by row category labels.
    #[must_use]
    pub fn to_tsv(&self) -> String {
        let mut lines = Vec::with_capacity(self.counts.len() + 1);
        let mut hdr = vec![self.row_factor_title];
        hdr.extend_from_slice(&self.column_titles);
        lines.push(hdr.join("\t"));
        for (r, row) in self.counts.iter().enumerate() {
            let mut cols = vec![self.row_titles.get(r).copied().unwrap_or("").to_owned()];
            for &n in row {
                cols.push(format!("{n}"));
            }
            lines.push(cols.join("\t"));
        }
        lines.join("\n")
    }

    /// Build a [`ContingencyTable`]-backed [`DataSheet`] for headless `Session` oracle verification.
    pub fn to_data_sheet(&self, sheet_id: &str) -> Result<DataSheet, DocError> {
        let mut tbl = ContingencyTable::new(self.column_titles.len(), self.counts.len())?;
        tbl.row_factor_title = self.row_factor_title.to_owned();
        tbl.column_factor_title = self.column_factor_title.to_owned();
        tbl.row_titles = self.row_titles.iter().map(|s| (*s).to_owned()).collect();
        for (c, title) in self.column_titles.iter().enumerate() {
            tbl.data_sets[c].title = (*title).to_owned();
        }
        for (r, row) in self.counts.iter().enumerate() {
            for (c, &n) in row.iter().enumerate() {
                tbl.data_sets[c].cells[r][0] = Cell::number(n as f64);
            }
        }
        DataSheet::new(sheet_id, self.title, TableArchetype::Contingency(tbl))
    }
}

/// Clean-room nested hierarchical treatment -> biological unit -> technical replicate fixture.
#[derive(Debug, Clone, PartialEq)]
pub struct NestedFixture {
    /// Sheet display title.
    pub title: &'static str,
    /// Top-level treatment group titles.
    pub group_titles: Vec<&'static str>,
    /// Technical replicate values: `values[group_idx][bio_unit_idx][tech_row_idx]`.
    pub values: Vec<Vec<Vec<f64>>>,
}

impl NestedFixture {
    /// Number of biological units per treatment group.
    #[must_use]
    pub fn bio_units_per_group(&self) -> usize {
        self.values.first().map_or(0, Vec::len)
    }

    /// Number of technical replicate rows per biological unit.
    #[must_use]
    pub fn tech_rows(&self) -> usize {
        self.values.first().and_then(|g| g.first()).map_or(0, Vec::len)
    }

    /// Build a [`NestedTable`]-backed [`DataSheet`] for headless `Session` oracle verification.
    pub fn to_data_sheet(&self, sheet_id: &str) -> Result<DataSheet, DocError> {
        let mut tbl = NestedTable::new(self.group_titles.len(), self.bio_units_per_group(), self.tech_rows())?;
        for (g, title) in self.group_titles.iter().enumerate() {
            tbl.groups[g].title = (*title).to_owned();
            for u in 0..self.bio_units_per_group() {
                tbl.groups[g].subcolumn_titles[u] = format!("{title}:{}", u + 1);
                for r in 0..self.tech_rows() {
                    tbl.groups[g].cells[r][u] = Cell::number(self.values[g][u][r]);
                }
            }
        }
        DataSheet::new(sheet_id, self.title, TableArchetype::Nested(tbl))
    }
}

/// Clean-room two-way or `2 × 2 × K` three-way grouped factorial fixture.
#[derive(Debug, Clone, PartialEq)]
pub struct GroupedFixture {
    /// Sheet display title.
    pub title: &'static str,
    /// Row factor title (*A*).
    pub row_factor_title: &'static str,
    /// Primary column factor title (*B*).
    pub column_factor_title: &'static str,
    /// Row factor level labels.
    pub row_titles: Vec<&'static str>,
    /// Column dataset titles (`2+` for two-way, `4` for `2 × 2 × K` three-way).
    pub column_titles: Vec<&'static str>,
    /// Replicate subcolumn count per group cell.
    pub replicates: usize,
    /// Optional `2 × 2` three-way column-factor metadata.
    pub three_way: Option<ThreeWaySpec>,
    /// Replicate values: `values[row][col][subcol]`.
    pub values: Vec<Vec<Vec<f64>>>,
}

/// Optional three-way cross-classification labels for [`GroupedFixture`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThreeWaySpec {
    pub factor_1_name: &'static str,
    pub factor_1_levels: [&'static str; 2],
    pub factor_2_name: &'static str,
    pub factor_2_levels: [&'static str; 2],
}

impl GroupedFixture {
    /// Render as tab-separated rows with row labels and replicate columns.
    #[must_use]
    pub fn to_tsv(&self) -> String {
        let mut lines = Vec::with_capacity(self.values.len() + 1);
        let mut hdr = vec![self.row_factor_title.to_owned()];
        for col in &self.column_titles {
            for rep in 1..=self.replicates {
                hdr.push(format!("{col}_R{rep}"));
            }
        }
        lines.push(hdr.join("\t"));
        for (r, row) in self.values.iter().enumerate() {
            let mut cols = vec![self.row_titles.get(r).copied().unwrap_or("").to_owned()];
            for cell in row {
                for &v in cell {
                    cols.push(format!("{v}"));
                }
            }
            lines.push(cols.join("\t"));
        }
        lines.join("\n")
    }

    /// Build a [`GroupedTable`]-backed [`DataSheet`] for headless `Session` oracle verification.
    pub fn to_data_sheet(&self, sheet_id: &str) -> Result<DataSheet, DocError> {
        let mut tbl = GroupedTable::new(SubcolumnMode::RawReplicates(self.replicates), self.column_titles.len(), self.row_titles.len())?;
        tbl.row_factor_title = self.row_factor_title.to_owned();
        tbl.column_factor_title = self.column_factor_title.to_owned();
        tbl.row_titles = self.row_titles.iter().map(|s| (*s).to_owned()).collect();
        tbl.three_way_grouping = self.three_way.map(|tw| ThreeWayColumnGrouping {
            factor_1_name: tw.factor_1_name.to_owned(),
            factor_1_levels: [tw.factor_1_levels[0].to_owned(), tw.factor_1_levels[1].to_owned()],
            factor_2_name: tw.factor_2_name.to_owned(),
            factor_2_levels: [tw.factor_2_levels[0].to_owned(), tw.factor_2_levels[1].to_owned()],
        });
        for (c, title) in self.column_titles.iter().enumerate() {
            tbl.data_sets[c].title = (*title).to_owned();
        }
        for (r, row) in self.values.iter().enumerate() {
            for (c, cell) in row.iter().enumerate() {
                for (s, &v) in cell.iter().enumerate() {
                    tbl.data_sets[c].cells[r][s] = Cell::number(v);
                }
            }
        }
        DataSheet::new(sheet_id, self.title, TableArchetype::Grouped(tbl))
    }
}

/// Clean-room one-way column group fixture (`Column` workflows).
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnFixture {
    /// Sheet display title.
    pub title: &'static str,
    /// Column group titles.
    pub group_titles: Vec<&'static str>,
    /// Values per column (`columns[col][row]`).
    pub columns: Vec<Vec<f64>>,
}

impl ColumnFixture {
    /// Render as tab-separated columns.
    #[must_use]
    pub fn to_tsv(&self) -> String {
        let max_rows = self.columns.iter().map(Vec::len).max().unwrap_or(0);
        let mut lines = Vec::with_capacity(max_rows + 1);
        lines.push(self.group_titles.join("\t"));
        for r in 0..max_rows {
            let cells: Vec<String> = self.columns.iter().map(|col| col.get(r).map_or_else(String::new, |v| format!("{v}"))).collect();
            lines.push(cells.join("\t"));
        }
        lines.join("\n")
    }

    /// Build a [`ColumnTable`]-backed [`DataSheet`] for headless `Session` oracle verification.
    pub fn to_data_sheet(&self, sheet_id: &str) -> Result<DataSheet, DocError> {
        let rows = self.columns.iter().map(Vec::len).max().unwrap_or(1).max(1);
        let mut tbl = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, self.columns.len(), rows)?;
        for (c, title) in self.group_titles.iter().enumerate() {
            tbl.data_sets[c].title = (*title).to_owned();
            for (r, &v) in self.columns[c].iter().enumerate() {
                tbl.data_sets[c].cells[r][0] = Cell::number(v);
            }
        }
        DataSheet::new(sheet_id, self.title, TableArchetype::Column(tbl))
    }
}

/// Two-group Kaplan–Meier survival fixture with distinct hazard and right-censoring events.
#[must_use]
pub fn survival_two_group() -> SurvivalFixture {
    SurvivalFixture {
        title: "Survival Two-Arm Cohort",
        time_title: "Days",
        cohort_titles: vec!["Control", "Therapy"],
        rows: vec![
            (4.0, vec![Some(1), None]),
            (6.0, vec![Some(1), Some(0)]),
            (9.0, vec![Some(1), None]),
            (11.0, vec![Some(0), Some(1)]),
            (14.0, vec![Some(1), None]),
            (17.0, vec![Some(1), Some(0)]),
            (20.0, vec![Some(1), Some(1)]),
            (23.0, vec![Some(0), None]),
            (26.0, vec![Some(1), Some(1)]),
            (30.0, vec![None, Some(0)]),
            (34.0, vec![None, Some(1)]),
            (38.0, vec![None, Some(0)]),
            (42.0, vec![None, Some(1)]),
            (46.0, vec![None, Some(0)]),
        ],
    }
}

/// Three-arm survival dose-response fixture (`Vehicle`, `LowDose`, `HighDose`).
#[must_use]
pub fn survival_three_group() -> SurvivalFixture {
    SurvivalFixture {
        title: "Survival Three-Arm Study",
        time_title: "Weeks",
        cohort_titles: vec!["Vehicle", "LowDose", "HighDose"],
        rows: vec![
            (3.0, vec![Some(1), None, None]),
            (5.0, vec![Some(1), Some(1), None]),
            (7.0, vec![Some(1), None, Some(0)]),
            (9.0, vec![Some(0), Some(1), None]),
            (12.0, vec![Some(1), Some(0), Some(1)]),
            (15.0, vec![Some(1), Some(1), None]),
            (18.0, vec![None, Some(1), Some(0)]),
            (21.0, vec![None, Some(0), Some(1)]),
            (25.0, vec![None, Some(1), Some(0)]),
            (29.0, vec![None, None, Some(1)]),
            (33.0, vec![None, None, Some(0)]),
            (37.0, vec![None, None, Some(1)]),
            (41.0, vec![None, None, Some(0)]),
        ],
    }
}

/// Multivariate clinical time-to-event cohort (`Time`, `Status`, `Treatment`, `Age`, `Biomarker`)
/// for Cox proportional-hazards workflows.
#[must_use]
pub fn cox_ph_clinical() -> MvFixture {
    let rows = vec![
        (6.0, 1.0, "Control", 64.0, 3.8),
        (9.0, 1.0, "Control", 58.0, 3.2),
        (11.0, 1.0, "Control", 71.0, 4.1),
        (14.0, 0.0, "Control", 49.0, 2.5),
        (16.0, 1.0, "Control", 62.0, 3.4),
        (19.0, 1.0, "Control", 55.0, 2.9),
        (22.0, 0.0, "Control", 52.0, 2.3),
        (25.0, 1.0, "Control", 66.0, 3.1),
        (12.0, 1.0, "Therapy", 68.0, 3.6),
        (18.0, 0.0, "Therapy", 51.0, 2.1),
        (21.0, 1.0, "Therapy", 61.0, 2.8),
        (26.0, 0.0, "Therapy", 47.0, 1.9),
        (29.0, 1.0, "Therapy", 63.0, 2.7),
        (34.0, 0.0, "Therapy", 45.0, 1.7),
        (38.0, 1.0, "Therapy", 59.0, 2.4),
        (44.0, 0.0, "Therapy", 50.0, 1.8),
    ];
    MvFixture {
        title: "Cox PH Clinical Cohort",
        columns: vec![
            MvColumnSpec { title: "Time", role: VariableRole::Continuous, role_slug: "continuous" },
            MvColumnSpec { title: "Status", role: VariableRole::Continuous, role_slug: "continuous" },
            MvColumnSpec { title: "Treatment", role: VariableRole::Categorical, role_slug: "categorical" },
            MvColumnSpec { title: "Age", role: VariableRole::Continuous, role_slug: "continuous" },
            MvColumnSpec { title: "Biomarker", role: VariableRole::Continuous, role_slug: "continuous" },
        ],
        rows: rows
            .into_iter()
            .map(|(t, s, arm, age, bio)| {
                vec![MvCellValue::Number(t), MvCellValue::Number(s), MvCellValue::Text(arm), MvCellValue::Number(age), MvCellValue::Number(bio)]
            })
            .collect(),
    }
}

/// Well-conditioned Multiple Linear GLM fixture (`Response`, `PredictorA`, `PredictorB`, `Group`).
#[must_use]
pub fn glm_multiple_linear() -> MvFixture {
    let rows = vec![
        (18.4, 2.0, 1.0, "Ref"),
        (21.2, 3.5, 1.5, "Ref"),
        (23.1, 4.0, 1.0, "Ref"),
        (25.9, 5.5, 2.0, "Ref"),
        (29.0, 6.5, 1.5, "Ref"),
        (31.8, 8.0, 2.5, "Ref"),
        (34.5, 9.0, 2.0, "Ref"),
        (21.8, 2.5, 1.0, "Treated"),
        (25.4, 4.0, 1.2, "Treated"),
        (28.2, 5.0, 1.0, "Treated"),
        (31.6, 6.5, 1.5, "Treated"),
        (34.9, 7.5, 1.2, "Treated"),
        (38.7, 9.0, 1.8, "Treated"),
        (41.2, 10.0, 1.5, "Treated"),
    ];
    MvFixture {
        title: "GLM Multiple Linear",
        columns: vec![
            MvColumnSpec { title: "Response", role: VariableRole::Continuous, role_slug: "continuous" },
            MvColumnSpec { title: "PredictorA", role: VariableRole::Continuous, role_slug: "continuous" },
            MvColumnSpec { title: "PredictorB", role: VariableRole::Continuous, role_slug: "continuous" },
            MvColumnSpec { title: "Group", role: VariableRole::Categorical, role_slug: "categorical" },
        ],
        rows: rows
            .into_iter()
            .map(|(y, a, b, g)| vec![MvCellValue::Number(y), MvCellValue::Number(a), MvCellValue::Number(b), MvCellValue::Text(g)])
            .collect(),
    }
}

/// Binary logistic GLM fixture (`Event`, `Dose`, `BaselineMarker`, `Cohort`) with interior overlap
/// so IRLS/MLE converges without separation.
#[must_use]
pub fn glm_logistic() -> MvFixture {
    let rows = vec![
        (0.0, 1.0, 1.2, "CohortA"),
        (0.0, 1.8, 1.9, "CohortA"),
        (1.0, 2.5, 2.3, "CohortA"),
        (0.0, 3.0, 1.5, "CohortA"),
        (0.0, 3.8, 2.1, "CohortA"),
        (1.0, 4.4, 2.8, "CohortA"),
        (1.0, 5.5, 3.1, "CohortA"),
        (1.0, 6.8, 3.9, "CohortA"),
        (0.0, 1.2, 1.4, "CohortB"),
        (0.0, 2.2, 1.8, "CohortB"),
        (1.0, 3.4, 2.7, "CohortB"),
        (0.0, 4.1, 2.2, "CohortB"),
        (1.0, 5.0, 3.3, "CohortB"),
        (1.0, 5.9, 3.6, "CohortB"),
        (1.0, 6.5, 3.8, "CohortB"),
        (1.0, 7.6, 4.4, "CohortB"),
    ];
    MvFixture {
        title: "GLM Logistic Binary",
        columns: vec![
            MvColumnSpec { title: "Event", role: VariableRole::Continuous, role_slug: "continuous" },
            MvColumnSpec { title: "Dose", role: VariableRole::Continuous, role_slug: "continuous" },
            MvColumnSpec { title: "BaselineMarker", role: VariableRole::Continuous, role_slug: "continuous" },
            MvColumnSpec { title: "Cohort", role: VariableRole::Categorical, role_slug: "categorical" },
        ],
        rows: rows
            .into_iter()
            .map(|(ev, d, m, c)| vec![MvCellValue::Number(ev), MvCellValue::Number(d), MvCellValue::Number(m), MvCellValue::Text(c)])
            .collect(),
    }
}

/// Poisson count GLM fixture (`Counts`, `ExposureTime`, `DoseLevel`, `Arm`).
#[must_use]
pub fn glm_poisson() -> MvFixture {
    let rows = vec![
        (2.0, 1.0, 0.5, "Control"),
        (3.0, 1.2, 1.0, "Control"),
        (5.0, 1.5, 1.5, "Control"),
        (6.0, 1.5, 2.0, "Control"),
        (8.0, 1.8, 2.5, "Control"),
        (11.0, 2.0, 3.0, "Control"),
        (13.0, 2.1, 3.5, "Control"),
        (3.0, 1.0, 0.5, "Stimulated"),
        (5.0, 1.2, 1.0, "Stimulated"),
        (7.0, 1.4, 1.5, "Stimulated"),
        (10.0, 1.6, 2.0, "Stimulated"),
        (14.0, 1.8, 2.5, "Stimulated"),
        (17.0, 2.0, 3.0, "Stimulated"),
        (21.0, 2.2, 3.5, "Stimulated"),
    ];
    MvFixture {
        title: "GLM Poisson Counts",
        columns: vec![
            MvColumnSpec { title: "Counts", role: VariableRole::Continuous, role_slug: "continuous" },
            MvColumnSpec { title: "ExposureTime", role: VariableRole::Continuous, role_slug: "continuous" },
            MvColumnSpec { title: "DoseLevel", role: VariableRole::Continuous, role_slug: "continuous" },
            MvColumnSpec { title: "Arm", role: VariableRole::Categorical, role_slug: "categorical" },
        ],
        rows: rows
            .into_iter()
            .map(|(cnt, exp, dose, arm)| vec![MvCellValue::Number(cnt), MvCellValue::Number(exp), MvCellValue::Number(dose), MvCellValue::Text(arm)])
            .collect(),
    }
}

/// 4-parameter log-logistic dose-response fixture (`Log10(M)` *X* × two triplicate *Y* compounds).
#[must_use]
pub fn dose_response_4pl() -> XyReplicateFixture {
    let data: &[(f64, [f64; 3], [f64; 3])] = &[
        (-9.0, [5.1, 5.4, 4.9], [6.2, 5.9, 6.4]),
        (-8.0, [6.0, 5.7, 6.2], [6.5, 6.8, 6.3]),
        (-7.0, [8.4, 8.1, 8.8], [7.2, 7.6, 7.0]),
        (-6.0, [28.2, 29.1, 27.5], [10.4, 10.9, 9.9]),
        (-5.0, [76.5, 75.8, 77.4], [24.0, 24.8, 23.4]),
        (-4.0, [97.1, 96.4, 97.8], [62.5, 61.8, 63.3]),
        (-3.0, [99.6, 100.2, 99.1], [90.2, 89.6, 91.0]),
        (-2.0, [100.1, 99.7, 100.4], [95.4, 96.0, 94.9]),
    ];
    XyReplicateFixture {
        title: "4PL Dose Response",
        x_title: "Log10(Dose M)",
        replicates: 3,
        series_titles: vec!["Agonist_A", "Agonist_B"],
        rows: data.iter().map(|&(x, a, b)| XySeriesRow { x, series_replicates: vec![a.to_vec(), b.to_vec()] }).collect(),
    }
}

/// Asymmetric 5-parameter log-logistic dose-response fixture.
#[must_use]
pub fn dose_response_5pl() -> XyReplicateFixture {
    // Generated with Bottom=4, Top=104, LogXb=-5.4, Hill=1.35, Symmetry=1.8 plus small deterministic replicate spreads.
    let data: &[(f64, [f64; 3])] = &[
        (-9.0, [4.2, 4.0, 4.4]),
        (-8.0, [5.1, 4.8, 5.3]),
        (-7.0, [11.4, 11.9, 10.9]),
        (-6.0, [42.1, 41.4, 42.9]),
        (-5.0, [84.6, 85.3, 83.9]),
        (-4.0, [99.2, 98.7, 99.6]),
        (-3.0, [103.1, 103.6, 102.8]),
        (-2.0, [103.9, 104.2, 103.7]),
        (-1.0, [104.1, 103.9, 104.3]),
    ];
    XyReplicateFixture {
        title: "5PL Asymmetric Dose Response",
        x_title: "Log10(Conc M)",
        replicates: 3,
        series_titles: vec!["Asymmetric_Series"],
        rows: data.iter().map(|&(x, y)| XySeriesRow { x, series_replicates: vec![y.to_vec()] }).collect(),
    }
}

/// qPCR Livak–Schmittgen $2^{-\Delta\Delta C_t}$ source fixture (`Group`, `TargetCt`, `RefCt`, `Well`).
///
/// Calibrator mean $\Delta C_t = 6.0$; Perturbed mean $\Delta C_t = 4.0$ ($\Delta\Delta C_t = -2.0$, fold change $= 4.0$).
#[must_use]
pub fn qpcr_ddct() -> MvFixture {
    let rows = vec![
        ("Calibrator", 24.1, 18.1, "A1"),
        ("Calibrator", 23.9, 17.9, "A2"),
        ("Calibrator", 24.2, 18.2, "A3"),
        ("Calibrator", 23.8, 17.8, "A4"),
        ("Calibrator", 24.0, 18.0, "A5"),
        ("Calibrator", 24.0, 18.0, "A6"),
        ("Perturbed", 22.1, 18.1, "B1"),
        ("Perturbed", 21.8, 17.9, "B2"),
        ("Perturbed", 22.2, 18.1, "B3"),
        ("Perturbed", 21.9, 17.9, "B4"),
        ("Perturbed", 22.0, 18.0, "B5"),
        ("Perturbed", 22.0, 18.0, "B6"),
    ];
    MvFixture {
        title: "qPCR ddCt Assay",
        columns: vec![
            MvColumnSpec { title: "Group", role: VariableRole::Categorical, role_slug: "categorical" },
            MvColumnSpec { title: "TargetCt", role: VariableRole::Continuous, role_slug: "continuous" },
            MvColumnSpec { title: "RefCt", role: VariableRole::Continuous, role_slug: "continuous" },
            MvColumnSpec { title: "Well", role: VariableRole::Label, role_slug: "label" },
        ],
        rows: rows
            .into_iter()
            .map(|(grp, target, hk, well)| {
                vec![MvCellValue::Text(grp), MvCellValue::Number(target), MvCellValue::Number(hk), MvCellValue::Text(well)]
            })
            .collect(),
    }
}

/// Michaelis–Menten enzyme kinetics fixture comparing `WildType` (`Vmax=120, Km=8`) and `Mutant` (`Vmax=84, Km=18`).
#[must_use]
pub fn michaelis_menten() -> XyReplicateFixture {
    let data: &[(f64, [f64; 2], [f64; 2])] = &[
        (0.5, [7.1, 6.9], [2.2, 2.4]),
        (1.0, [13.2, 13.5], [4.5, 4.3]),
        (2.0, [24.1, 23.9], [8.3, 8.5]),
        (5.0, [46.2, 46.0], [18.1, 18.4]),
        (10.0, [66.8, 66.5], [29.9, 30.2]),
        (25.0, [91.1, 90.7], [48.7, 49.0]),
        (50.0, [103.5, 103.2], [61.6, 62.0]),
        (100.0, [111.0, 111.3], [71.1, 71.4]),
    ];
    XyReplicateFixture {
        title: "Michaelis-Menten Kinetics",
        x_title: "Substrate [S] uM",
        replicates: 2,
        series_titles: vec!["WildType", "Mutant"],
        rows: data.iter().map(|&(s, wt, mut_v)| XySeriesRow { x: s, series_replicates: vec![wt.to_vec(), mut_v.to_vec()] }).collect(),
    }
}

/// `2 × 2` contingency fixture with strong association (`OR ≈ 8.14`).
#[must_use]
pub fn contingency_2x2() -> ContingencyFixture {
    ContingencyFixture {
        title: "Contingency 2x2 Exposure",
        row_factor_title: "Exposure",
        column_factor_title: "Outcome",
        row_titles: vec!["Exposed", "Unexposed"],
        column_titles: vec!["Responder", "NonResponder"],
        counts: vec![vec![18, 6], vec![7, 19]],
    }
}

/// `2 × 3` ordered contingency fixture suitable for Chi-square test for trend.
#[must_use]
pub fn contingency_2x3_trend() -> ContingencyFixture {
    ContingencyFixture {
        title: "Contingency 2x3 Dose Trend",
        row_factor_title: "Outcome",
        column_factor_title: "DoseTier",
        row_titles: vec!["Improved", "Unchanged"],
        column_titles: vec!["Low", "Medium", "High"],
        counts: vec![vec![6, 14, 24], vec![22, 14, 6]],
    }
}

/// `3 × 3` symmetric agreement / association contingency fixture.
#[must_use]
pub fn contingency_3x3() -> ContingencyFixture {
    ContingencyFixture {
        title: "Contingency 3x3 Stratification",
        row_factor_title: "AssayGrade",
        column_factor_title: "ClinicalStage",
        row_titles: vec!["Mild", "Moderate", "Severe"],
        column_titles: vec!["StageI", "StageII", "StageIII"],
        counts: vec![vec![20, 7, 3], vec![6, 19, 5], vec![2, 5, 22]],
    }
}

/// Three well-separated 3D clusters (`12` labeled samples) for hierarchical & K-means clustering workflows.
#[must_use]
pub fn clustering_three_clusters() -> MvFixture {
    let rows = vec![
        ("S01", 2.0, 2.2, 1.9),
        ("S02", 2.4, 1.8, 2.1),
        ("S03", 1.7, 2.3, 2.2),
        ("S04", 2.2, 2.0, 1.7),
        ("S05", 11.8, 10.9, 3.1),
        ("S06", 12.2, 11.3, 2.8),
        ("S07", 11.6, 11.1, 3.3),
        ("S08", 12.4, 10.7, 2.9),
        ("S09", 4.1, 14.0, 12.9),
        ("S10", 3.8, 14.4, 13.3),
        ("S11", 4.3, 13.7, 13.1),
        ("S12", 3.9, 14.2, 12.7),
    ];
    MvFixture {
        title: "Clustering Three Clusters",
        columns: vec![
            MvColumnSpec { title: "Sample", role: VariableRole::Label, role_slug: "label" },
            MvColumnSpec { title: "FeatureX", role: VariableRole::Continuous, role_slug: "continuous" },
            MvColumnSpec { title: "FeatureY", role: VariableRole::Continuous, role_slug: "continuous" },
            MvColumnSpec { title: "FeatureZ", role: VariableRole::Continuous, role_slug: "continuous" },
        ],
        rows: rows
            .into_iter()
            .map(|(lbl, x, y, z)| vec![MvCellValue::Text(lbl), MvCellValue::Number(x), MvCellValue::Number(y), MvCellValue::Number(z)])
            .collect(),
    }
}

/// Nested one-way hierarchy (`3` groups × `3` biological units × `4` technical replicates).
#[must_use]
pub fn nested_one_way() -> NestedFixture {
    NestedFixture {
        title: "Nested Three-Group Litters",
        group_titles: vec!["Vehicle", "LowDose", "HighDose"],
        values: vec![
            vec![vec![10.2, 10.4, 10.1, 10.3], vec![11.0, 11.2, 10.9, 11.1], vec![9.8, 9.9, 9.7, 10.0]],
            vec![vec![14.1, 14.3, 14.0, 14.2], vec![14.8, 15.0, 14.7, 14.9], vec![13.7, 13.9, 13.6, 13.8]],
            vec![vec![19.5, 19.8, 19.4, 19.6], vec![20.3, 20.5, 20.1, 20.4], vec![19.0, 19.2, 18.9, 19.1]],
        ],
    }
}

/// Two-way grouped factorial fixture (`3` time points × `2` treatments × `3` replicates).
#[must_use]
pub fn grouped_two_way() -> GroupedFixture {
    GroupedFixture {
        title: "Grouped Two-Way Timecourse",
        row_factor_title: "TimePoint",
        column_factor_title: "Treatment",
        row_titles: vec!["0h", "6h", "24h"],
        column_titles: vec!["Vehicle", "Inhibitor"],
        replicates: 3,
        three_way: None,
        values: vec![
            vec![vec![12.0, 12.4, 11.8], vec![12.1, 11.9, 12.3]],
            vec![vec![14.2, 14.5, 13.9], vec![18.8, 19.2, 18.5]],
            vec![vec![15.1, 14.8, 15.4], vec![27.4, 28.0, 26.9]],
        ],
    }
}

/// `2 × 2 × 2` three-way grouped factorial fixture (`2` doses × `4` column combinations × `3` replicates).
#[must_use]
pub fn grouped_three_way() -> GroupedFixture {
    GroupedFixture {
        title: "Grouped Three-Way Factorial",
        row_factor_title: "Dose",
        column_factor_title: "TreatmentSex",
        row_titles: vec!["Low", "High"],
        column_titles: vec!["Veh_F", "Veh_M", "Drug_F", "Drug_M"],
        replicates: 3,
        three_way: Some(ThreeWaySpec {
            factor_1_name: "Treatment",
            factor_1_levels: ["Vehicle", "Drug"],
            factor_2_name: "Sex",
            factor_2_levels: ["Female", "Male"],
        }),
        values: vec![
            vec![vec![10.2, 10.5, 9.9], vec![11.4, 11.8, 11.2], vec![14.8, 15.1, 14.5], vec![16.2, 16.6, 15.9]],
            vec![vec![10.8, 11.1, 10.6], vec![12.0, 12.4, 11.8], vec![22.5, 23.0, 22.1], vec![25.4, 25.9, 24.8]],
        ],
    }
}

/// Three-group column assay (`Baseline`, `Compound_Low`, `Compound_High`, `6` replicates each).
#[must_use]
pub fn column_three_group() -> ColumnFixture {
    ColumnFixture {
        title: "Column Three-Group Assay",
        group_titles: vec!["Baseline", "Compound_Low", "Compound_High"],
        columns: vec![vec![10.1, 10.8, 9.7, 11.2, 10.4, 9.9], vec![14.5, 15.1, 13.9, 14.8, 15.4, 14.2], vec![21.2, 22.0, 20.6, 21.8, 22.4, 20.9]],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn verify_mv(fx: &MvFixture, id: &str) -> Result<(), String> {
        let sheet = fx.to_data_sheet(id).map_err(|e| e.to_string())?;
        let TableArchetype::MultipleVariables(mv) = &sheet.archetype else {
            return Err(String::from("expected MultipleVariables"));
        };
        if mv.variables.len() != fx.columns.len() {
            return Err(format!("column count mismatch in {id}"));
        }
        for (c, spec) in fx.columns.iter().enumerate() {
            if mv.variables[c].role != spec.role {
                return Err(format!("role mismatch at col {c} in {id}"));
            }
            if mv.variables[c].cells.len() != fx.rows.len() {
                return Err(format!("row count mismatch at col {c} in {id}"));
            }
        }
        if fx.to_tsv().lines().count() != fx.rows.len() + 1 {
            return Err(format!("TSV line count mismatch in {id}"));
        }
        Ok(())
    }

    #[test]
    fn all_wf_datasets_build_valid_sheets_and_tsv() -> Result<(), String> {
        for (surv, id) in [(survival_two_group(), "surv-2"), (survival_three_group(), "surv-3")] {
            let sheet = surv.to_data_sheet(id).map_err(|e| e.to_string())?;
            let TableArchetype::Survival(s) = &sheet.archetype else {
                return Err(String::from("expected Survival"));
            };
            if s.cohorts.len() < 2 || s.times.is_empty() {
                return Err(format!("invalid survival fixture {id}"));
            }
            let event_count = surv.rows.iter().flat_map(|(_, cs)| cs.iter()).filter(|x| **x == Some(1)).count();
            let censor_count = surv.rows.iter().flat_map(|(_, cs)| cs.iter()).filter(|x| **x == Some(0)).count();
            if event_count == 0 || censor_count == 0 {
                return Err(format!("survival fixture {id} needs both events and censored rows"));
            }
            if surv.to_tsv().lines().count() != surv.rows.len() + 1 {
                return Err(format!("survival TSV mismatch {id}"));
            }
        }

        verify_mv(&cox_ph_clinical(), "cox-ph")?;
        verify_mv(&glm_multiple_linear(), "glm-lin")?;
        verify_mv(&glm_logistic(), "glm-log")?;
        verify_mv(&glm_poisson(), "glm-poi")?;
        verify_mv(&qpcr_ddct(), "qpcr")?;
        verify_mv(&clustering_three_clusters(), "cluster")?;

        for (xy, id) in [(dose_response_4pl(), "xy-4pl"), (dose_response_5pl(), "xy-5pl"), (michaelis_menten(), "xy-mm")] {
            let sheet = xy.to_data_sheet(id).map_err(|e| e.to_string())?;
            let TableArchetype::Xy(tbl) = &sheet.archetype else {
                return Err(String::from("expected Xy"));
            };
            if tbl.data_sets.is_empty() || tbl.x_cells.len() != xy.rows.len() {
                return Err(format!("invalid XY fixture {id}"));
            }
            if xy.to_tsv().lines().count() != xy.rows.len() + 1 {
                return Err(format!("XY TSV mismatch {id}"));
            }
        }

        for (ct, id) in [(contingency_2x2(), "ct-2x2"), (contingency_2x3_trend(), "ct-2x3"), (contingency_3x3(), "ct-3x3")] {
            let sheet = ct.to_data_sheet(id).map_err(|e| e.to_string())?;
            if !matches!(sheet.archetype, TableArchetype::Contingency(_)) {
                return Err(format!("expected Contingency in {id}"));
            }
            if ct.to_tsv().lines().count() != ct.counts.len() + 1 {
                return Err(format!("Contingency TSV mismatch {id}"));
            }
        }

        let nst = nested_one_way();
        let nst_sheet = nst.to_data_sheet("nst").map_err(|e| e.to_string())?;
        let TableArchetype::Nested(nt) = &nst_sheet.archetype else {
            return Err(String::from("expected Nested"));
        };
        if nt.groups.len() != 3 || nt.groups[0].biological_unit_count() != 3 || nt.groups[0].cells.len() != 4 {
            return Err(String::from("nested fixture dimensions mismatch"));
        }

        for (grp, id, expect_three_way) in [(grouped_two_way(), "grp-2", false), (grouped_three_way(), "grp-3", true)] {
            let sheet = grp.to_data_sheet(id).map_err(|e| e.to_string())?;
            let TableArchetype::Grouped(g) = &sheet.archetype else {
                return Err(String::from("expected Grouped"));
            };
            if g.three_way_grouping.is_some() != expect_three_way {
                return Err(format!("grouped three_way mismatch in {id}"));
            }
            if grp.to_tsv().lines().count() != grp.values.len() + 1 {
                return Err(format!("grouped TSV mismatch {id}"));
            }
        }

        let col = column_three_group();
        let col_sheet = col.to_data_sheet("col-3").map_err(|e| e.to_string())?;
        if !matches!(col_sheet.archetype, TableArchetype::Column(_)) {
            return Err(String::from("expected Column"));
        }
        if col.to_tsv().lines().count() != 7 {
            return Err(String::from("column TSV line count mismatch"));
        }
        Ok(())
    }
}
