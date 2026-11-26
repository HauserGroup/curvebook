#![forbid(unsafe_code)]

//! Clean-room synthetic datasets for `R13` (`WF-51`–`WF-65`) acceptance testing.
//!
//! All tables, titles, numeric series, and raster patterns in this module are original
//! synthetic fixtures constructed specifically for CurveBook verification.

use curvebook_doc::{SubcolumnMode, TableKind};

/// Triplicate X-Y dose-response row for `WF-51`.
#[derive(Debug, Clone, PartialEq)]
pub struct XyTriplicatePoint {
    /// Log10 molar concentration.
    pub log_dose: f64,
    /// Three technical replicate activity measurements.
    pub replicates: [f64; 3],
}

/// Clean-room dataset for `WF-51` (ROUT outlier, dual unknown interpolation, 3PL/4PL, Normalize).
#[derive(Debug, Clone, PartialEq)]
pub struct Wf51Dataset {
    /// Table sheet title.
    pub title: &'static str,
    /// Table schema kind (`TableKind::Xy`).
    pub kind: TableKind,
    /// Subcolumn structure (`RawReplicates(3)`).
    pub subcolumns: SubcolumnMode,
    /// Primary dataset column title.
    pub data_set_title: &'static str,
    /// Dose-response rows with one injected outlier.
    pub known_rows: Vec<XyTriplicatePoint>,
    /// Unknown rows (`X` empty, triplicate `Y`).
    pub unknown_rows: Vec<[f64; 3]>,
    /// Zero-based row index of the injected outlier.
    pub outlier_row: usize,
    /// Zero-based replicate subcolumn index of the injected outlier.
    pub outlier_subcol: usize,
}

/// Single dose-response series for `WF-52`.
#[derive(Debug, Clone, PartialEq)]
pub struct Wf52Series {
    /// Series column title.
    pub title: &'static str,
    /// `(log_dose, percent_response)` pairs.
    pub rows: Vec<(f64, f64)>,
}

/// Clean-room sparse-plateau 4PL dataset for `WF-52`.
#[derive(Debug, Clone, PartialEq)]
pub struct Wf52Dataset {
    /// Table sheet title.
    pub title: &'static str,
    /// Two sparse-plateau agonist curves.
    pub series: [Wf52Series; 2],
}

/// Single observation row for `WF-53` 3-arm survival (may represent a blank spacer row).
#[derive(Debug, Clone, PartialEq)]
pub struct SurvivalEntry {
    /// Elapsed days (`None` represents an intentional blank spacer row).
    pub elapsed_days: Option<f64>,
    /// Status code (`1` event, `0` censored, `None` empty cell) per cohort.
    pub status: [Option<u8>; 3],
}

/// Clean-room 3-arm survival dataset for `WF-53`.
#[derive(Debug, Clone, PartialEq)]
pub struct Wf53Dataset {
    /// Table sheet title.
    pub title: &'static str,
    /// Three cohort names (`Vehicle` 100%-censored + two active treatment arms).
    pub cohorts: [&'static str; 3],
    /// Elapsed-time observations including blank spacer rows.
    pub rows: Vec<SurvivalEntry>,
}

/// Summary point (`Mean`, `SD`, `N`) for `WF-54`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeanSdNPoint {
    /// Arithmetic sample mean.
    pub mean: f64,
    /// Sample standard deviation.
    pub sd: f64,
    /// Replicate count.
    pub n: f64,
}

/// Clean-room dual-axis `MeanSdN` dataset for `WF-54`.
#[derive(Debug, Clone, PartialEq)]
pub struct Wf54Dataset {
    /// Table sheet title.
    pub title: &'static str,
    /// Two series titles (`Fluorescence` decay, `Metabolite` bars).
    pub series_titles: [&'static str; 2],
    /// Sampling times in hours.
    pub x_hours: Vec<f64>,
    /// Left-Y fluorescence decay series (`Mean`, `SD`, `N`).
    pub fluorescence: Vec<MeanSdNPoint>,
    /// Right-Y metabolite concentration series (`Mean`, `SD`, `N`).
    pub metabolite: Vec<MeanSdNPoint>,
}

/// Clean-room multi-site grouped cohort dataset for `WF-55` (12 sites × 4 quarters).
#[derive(Debug, Clone, PartialEq)]
pub struct Wf55Dataset {
    /// Table sheet title.
    pub title: &'static str,
    /// Quarter column titles (`4`).
    pub col_titles: Vec<&'static str>,
    /// Site row titles (`12`).
    pub row_titles: Vec<&'static str>,
    /// Site × quarter readout matrix.
    pub rows: Vec<Vec<f64>>,
}

/// Clean-room exponential-growth XY dataset for `WF-56`.
#[derive(Debug, Clone, PartialEq)]
pub struct Wf56Dataset {
    /// Table sheet title.
    pub title: &'static str,
    /// Pairs `(X, Y)` covering lag (`X < 10`) and log-linear (`X >= 10`) phases.
    pub rows: Vec<(f64, f64)>,
}

/// Clean-room synthetic RGBA raster schematic + small assay curve for `WF-57`.
#[derive(Debug, Clone, PartialEq)]
pub struct Wf57Dataset {
    /// Width of synthetic PNG schematic in pixels.
    pub width_px: u32,
    /// Height of synthetic PNG schematic in pixels.
    pub height_px: u32,
    /// Tightly packed RGBA bytes (`width_px * height_px * 4`).
    pub rgba: Vec<u8>,
    /// Companion XY trace points for panel composition.
    pub assay_xy: Vec<(f64, f64)>,
}

/// Clean-room 4-group Column dataset for `WF-58`.
#[derive(Debug, Clone, PartialEq)]
pub struct Wf58Dataset {
    /// Table sheet title.
    pub title: &'static str,
    /// Column group titles (`4`).
    pub group_titles: [&'static str; 4],
    /// Six observation rows across four groups.
    pub rows: Vec<[f64; 4]>,
}

/// Clean-room paired cohort + linearized inset dataset for `WF-59`.
#[derive(Debug, Clone, PartialEq)]
pub struct Wf59Dataset {
    /// Table sheet title.
    pub title: &'static str,
    /// Before/after column titles.
    pub paired_cols: [&'static str; 2],
    /// Eight matched subject pairs.
    pub paired_rows: Vec<[f64; 2]>,
    /// Six linearized inset `(X, Y)` pairs.
    pub inset_xy: Vec<(f64, f64)>,
}

/// Clean-room linear + outlier XY dataset for `WF-60`.
#[derive(Debug, Clone, PartialEq)]
pub struct Wf60Dataset {
    /// Table sheet title.
    pub title: &'static str,
    /// Rows with six collinear points plus two outlier rows.
    pub rows: Vec<(f64, f64)>,
    /// Row indices of the two outlier rows (`[6, 7]`).
    pub outlier_rows: [usize; 2],
}

/// Clean-room 13-biomarker grouped dataset for `WF-61`.
#[derive(Debug, Clone, PartialEq)]
pub struct Wf61Dataset {
    /// Table sheet title.
    pub title: &'static str,
    /// Thirteen multi-character biomarker column titles.
    pub analyte_titles: Vec<&'static str>,
    /// Six condition row titles.
    pub row_titles: Vec<&'static str>,
    /// Six condition rows × thirteen analytes spanning multi-order scales.
    pub rows: Vec<Vec<f64>>,
}

/// Clean-room five-design trial fixture for `WF-62` (`Nested`, `Paired`, `2-Way RM`).
#[derive(Debug, Clone, PartialEq)]
pub struct Wf62Dataset {
    /// Nested table title.
    pub nested_title: &'static str,
    /// Paired column table title.
    pub paired_title: &'static str,
    /// Longitudinal repeated-measures grouped table title.
    pub rm_title: &'static str,
    /// Two treatment arms for the nested table.
    pub nested_groups: [&'static str; 2],
    /// Biological subjects per arm (`4`).
    pub nested_subjects_per_group: usize,
    /// Technical replicates per subject (`3`).
    pub nested_reps_per_subject: usize,
    /// `[group][subject][rep]` nested measurements.
    pub nested_values: Vec<Vec<Vec<f64>>>,
    /// Eight matched subject pairs `[baseline, treated]`.
    pub paired_rows: Vec<[f64; 2]>,
    /// Three longitudinal timepoint labels.
    pub rm_timepoints: [&'static str; 3],
    /// Two-factor grouped rows (`3` timepoints × `8` flat subject subcolumns = `2` groups × `4` subjects).
    pub rm_rows: Vec<Vec<f64>>,
}

/// Clean-room hazard-ratio forest plot dataset for `WF-63`.
#[derive(Debug, Clone, PartialEq)]
pub struct Wf63Dataset {
    /// Table sheet title.
    pub title: &'static str,
    /// Six synthetic clinical trial names.
    pub study_titles: Vec<&'static str>,
    /// `[Estimate, LowerCI, UpperCI]` triplets per trial.
    pub entries: Vec<[f64; 3]>,
}

/// Clean-room demographic pyramid dataset for `WF-64`.
#[derive(Debug, Clone, PartialEq)]
pub struct Wf64Dataset {
    /// Table sheet title.
    pub title: &'static str,
    /// Seven age-band row titles (`"0-14"` to `"75+"`).
    pub band_titles: Vec<&'static str>,
    /// Negative left cohort and positive right cohort titles.
    pub cohort_titles: [&'static str; 2],
    /// Signed counts `[left_negative, right_positive]` per age band.
    pub rows: Vec<[f64; 2]>,
}

/// Clean-room longitudinal spaghetti dataset for `WF-65`.
#[derive(Debug, Clone, PartialEq)]
pub struct Wf65Dataset {
    /// Table sheet title.
    pub title: &'static str,
    /// Longitudinal follow-up weeks.
    pub weeks: Vec<f64>,
    /// Two treatment cohort names.
    pub cohort_titles: [&'static str; 2],
    /// Per-row `[cohort_0_4reps, cohort_1_4reps]` trajectories.
    pub replicate_trajectories: Vec<[Vec<f64>; 2]>,
    /// Single-Y subject column titles (`4` subjects).
    pub subject_titles: Vec<&'static str>,
    /// Per-row values across the 4 individual subject traces.
    pub single_y_subjects: Vec<Vec<f64>>,
}

/// Construct synthetic triplicate XY inhibitor dataset with an injected outlier (`WF-51`).
#[must_use]
pub fn wf51_inhibitor_triplicate_xy() -> Wf51Dataset {
    Wf51Dataset {
        title: "Inhibitor Dose Assay WF-51",
        kind: TableKind::Xy,
        subcolumns: SubcolumnMode::RawReplicates(3),
        data_set_title: "Inhibitor K-108",
        known_rows: vec![
            XyTriplicatePoint { log_dose: -9.0, replicates: [98.4, 99.6, 100.2] },
            XyTriplicatePoint { log_dose: -8.5, replicates: [95.8, 96.4, 97.2] },
            XyTriplicatePoint { log_dose: -8.0, replicates: [88.9, 89.5, 90.1] },
            XyTriplicatePoint { log_dose: -7.5, replicates: [69.5, 70.4, 178.0] },
            XyTriplicatePoint { log_dose: -7.0, replicates: [40.1, 41.0, 41.3] },
            XyTriplicatePoint { log_dose: -6.5, replicates: [18.4, 19.1, 19.6] },
            XyTriplicatePoint { log_dose: -6.0, replicates: [8.9, 9.4, 9.8] },
            XyTriplicatePoint { log_dose: -5.5, replicates: [5.9, 6.2, 6.7] },
        ],
        unknown_rows: vec![[52.5, 55.0, 50.5], [29.0, 31.5, 27.5]],
        outlier_row: 3,
        outlier_subcol: 2,
    }
}

/// Construct synthetic sparse-plateau agonist dataset (`WF-52`).
///
/// `Ligand Alpha` spans the upper plateau (~91% of `Top = 100`) while `Ligand Beta` is
/// right-shifted (`LogEC50 > -6.0`, max response ~33.6%) with no observed upper asymptote,
/// so unconstrained per-series 4PL is ambiguous/extrapolated while global sharing of `Top`
/// and `HillSlope` with `Bottom = 0` genuinely recovers identifiability (`AUD-13-04`).
#[must_use]
pub fn wf52_sparse_agonist_4pl_xy() -> Wf52Dataset {
    Wf52Dataset {
        title: "Sparse Agonist 4PL WF-52",
        series: [
            Wf52Series { title: "Ligand Alpha", rows: vec![(-9.0, 1.2), (-8.0, 9.2), (-7.0, 50.4), (-6.5, 76.2), (-6.0, 90.8)] },
            Wf52Series { title: "Ligand Beta", rows: vec![(-9.0, 0.8), (-8.0, 1.9), (-7.0, 6.4), (-6.5, 15.2), (-6.0, 33.6)] },
        ],
    }
}

/// Construct synthetic 3-arm numeric survival table with 100%-censored Vehicle + spacer rows (`WF-53`).
#[must_use]
pub fn wf53_three_arm_survival() -> Wf53Dataset {
    Wf53Dataset {
        title: "Three-Arm Survival WF-53",
        cohorts: ["Vehicle", "Compound A", "Compound B"],
        rows: vec![
            SurvivalEntry { elapsed_days: Some(6.0), status: [Some(0), Some(1), Some(0)] },
            SurvivalEntry { elapsed_days: Some(11.0), status: [Some(0), Some(1), Some(0)] },
            SurvivalEntry { elapsed_days: None, status: [None, None, None] },
            SurvivalEntry { elapsed_days: Some(16.0), status: [Some(0), Some(1), Some(1)] },
            SurvivalEntry { elapsed_days: Some(22.0), status: [Some(0), Some(1), Some(0)] },
            SurvivalEntry { elapsed_days: None, status: [None, None, None] },
            SurvivalEntry { elapsed_days: Some(29.0), status: [Some(0), Some(1), Some(1)] },
            SurvivalEntry { elapsed_days: Some(36.0), status: [Some(0), Some(0), Some(1)] },
            SurvivalEntry { elapsed_days: Some(44.0), status: [Some(0), Some(1), Some(0)] },
        ],
    }
}

/// Construct synthetic dual-axis `MeanSdN` time-course dataset (`WF-54`).
#[must_use]
pub fn wf54_dual_axis_mean_sd_n_xy() -> Wf54Dataset {
    Wf54Dataset {
        title: "Dual-Axis Kinetics WF-54",
        series_titles: ["Fluorescence Decay", "Metabolite Pool"],
        x_hours: vec![0.0, 2.0, 4.0, 8.0, 12.0, 24.0],
        fluorescence: vec![
            MeanSdNPoint { mean: 100.0, sd: 5.2, n: 6.0 },
            MeanSdNPoint { mean: 72.4, sd: 4.1, n: 6.0 },
            MeanSdNPoint { mean: 53.1, sd: 3.3, n: 6.0 },
            MeanSdNPoint { mean: 30.8, sd: 2.5, n: 6.0 },
            MeanSdNPoint { mean: 18.2, sd: 1.9, n: 6.0 },
            MeanSdNPoint { mean: 6.5, sd: 1.1, n: 6.0 },
        ],
        metabolite: vec![
            MeanSdNPoint { mean: 4.2, sd: 0.6, n: 6.0 },
            MeanSdNPoint { mean: 11.8, sd: 1.4, n: 6.0 },
            MeanSdNPoint { mean: 18.5, sd: 1.9, n: 6.0 },
            MeanSdNPoint { mean: 24.1, sd: 2.3, n: 6.0 },
            MeanSdNPoint { mean: 22.0, sd: 2.0, n: 6.0 },
            MeanSdNPoint { mean: 14.3, sd: 1.5, n: 6.0 },
        ],
    }
}

/// Construct synthetic 12-site × 4-quarter grouped cohort dataset (`WF-55`).
#[must_use]
pub fn wf55_multisite_grouped_cohort() -> Wf55Dataset {
    let row_titles = vec![
        "North Clinic",
        "South Clinic",
        "Harbor Lab",
        "Valley Unit",
        "Ridge Center",
        "Metro Ward",
        "Oakwood Site",
        "Riverfront",
        "Lakeside",
        "Hillcrest",
        "Midtown Hub",
        "Westfield",
    ];
    let rows = (0..12)
        .map(|s| {
            let base = 22.0 + (s as f64) * 3.1;
            vec![base, base + 4.5, base + 9.2, base + 12.8]
        })
        .collect();
    Wf55Dataset { title: "Multi-Site Screen WF-55", col_titles: vec!["Q1", "Q2", "Q3", "Q4"], row_titles, rows }
}

/// Construct synthetic lag-then-exponential growth dataset (`WF-56`).
#[must_use]
pub fn wf56_exponential_growth_xy() -> Wf56Dataset {
    let mut rows = vec![(2.0, 10.0), (4.0, 12.6), (6.0, 10.4), (8.0, 11.8)];
    for x in [10.0, 12.0, 14.0, 16.0, 18.0, 20.0] {
        rows.push((x, 10.0_f64.powf(0.12 * x)));
    }
    Wf56Dataset { title: "Growth Log-Range WF-56", rows }
}

/// Construct clean-room synthetic geometric RGBA pattern + XY trace (`WF-57`).
#[must_use]
pub fn wf57_schematic_rgba() -> Wf57Dataset {
    let width_px = 24u32;
    let height_px = 16u32;
    let mut rgba = Vec::with_capacity((width_px * height_px * 4) as usize);
    for y in 0..height_px {
        for x in 0..width_px {
            let border = x == 0 || y == 0 || x + 1 == width_px || y + 1 == height_px;
            let checker = ((x / 4) + (y / 4)) % 2 == 0;
            if border {
                rgba.extend_from_slice(&[24, 58, 104, 255]);
            } else if checker {
                rgba.extend_from_slice(&[72, 146, 208, 255]);
            } else {
                rgba.extend_from_slice(&[226, 238, 249, 255]);
            }
        }
    }
    Wf57Dataset { width_px, height_px, rgba, assay_xy: vec![(1.0, 12.0), (2.0, 24.0), (3.0, 37.5), (4.0, 51.0), (5.0, 64.5)] }
}

/// Construct clean-room 4-group Column dataset for descriptive slice & embed (`WF-58`).
#[must_use]
pub fn wf58_descriptive_column() -> Wf58Dataset {
    Wf58Dataset {
        title: "Cohort Readouts WF-58",
        group_titles: ["Sham", "Low Dose", "Mid Dose", "High Dose"],
        rows: vec![
            [18.2, 22.4, 29.1, 36.5],
            [19.5, 24.1, 31.4, 38.2],
            [17.8, 21.9, 28.6, 35.1],
            [20.1, 23.7, 32.0, 39.8],
            [18.9, 25.2, 30.3, 37.4],
            [19.2, 22.8, 29.8, 36.9],
        ],
    }
}

/// Construct clean-room paired cohort + linearized inset XY dataset (`WF-59`).
#[must_use]
pub fn wf59_paired_before_after_and_inset() -> Wf59Dataset {
    Wf59Dataset {
        title: "Paired Overlay WF-59",
        paired_cols: ["Baseline", "Post-Dose"],
        paired_rows: vec![[14.2, 21.5], [16.8, 24.9], [13.5, 19.7], [18.4, 28.1], [15.1, 23.2], [17.0, 25.8], [14.7, 22.4], [16.2, 24.1]],
        inset_xy: vec![(1.0, 2.1), (2.0, 4.0), (3.0, 6.2), (4.0, 8.1), (5.0, 10.3), (6.0, 12.0)],
    }
}

/// Construct clean-room subset linear XY dataset with two marked outlier rows (`WF-60`).
#[must_use]
pub fn wf60_subset_linear_with_outliers_xy() -> Wf60Dataset {
    Wf60Dataset {
        title: "Subset Regression WF-60",
        rows: vec![(1.0, 6.5), (2.0, 9.0), (3.0, 11.4), (4.0, 14.1), (5.0, 16.5), (6.0, 19.0), (7.0, 42.0), (8.0, 3.0)],
        outlier_rows: [6, 7],
    }
}

/// Construct clean-room 13-biomarker × 6-condition grouped dataset (`WF-61`).
#[must_use]
pub fn wf61_multivariate_biomarker_grouped() -> Wf61Dataset {
    let analyte_titles = vec![
        "Cytokine A",
        "Phospho-ERK",
        "Kinase Beta",
        "Chemokine 4",
        "Albumin Ratio",
        "Ferritin Index",
        "C-Reactive",
        "Troponin-S",
        "Fibrinogen",
        "Complement C3",
        "Lipase-B",
        "Transferrin",
        "Lactate Dehydrogenase",
    ];
    let row_titles = vec!["Control", "Acute-2h", "Acute-6h", "Peak-12h", "Recovery-24h", "Chronic-7d"];
    let rows = (0..6)
        .map(|r| {
            (0..13)
                .map(|c| {
                    let scale = 10.0_f64.powi(c % 3);
                    let pattern = 1.2 + ((r + 1) as f64) * (0.45 + 0.12 * ((c % 4) as f64));
                    pattern * scale
                })
                .collect()
        })
        .collect();
    Wf61Dataset { title: "Biomarker Matrix WF-61", analyte_titles, row_titles, rows }
}

/// Construct clean-room 5-design clinical trial dataset (`WF-62`).
#[must_use]
pub fn wf62_five_design_trial() -> Wf62Dataset {
    Wf62Dataset {
        nested_title: "Nested Trial WF-62",
        paired_title: "Matched Cohort WF-62",
        rm_title: "Longitudinal RM WF-62",
        nested_groups: ["Placebo", "Active Drug"],
        nested_subjects_per_group: 4,
        nested_reps_per_subject: 3,
        nested_values: vec![
            vec![vec![12.1, 12.6, 12.3], vec![14.0, 14.5, 14.2], vec![11.5, 11.9, 11.7], vec![13.2, 13.7, 13.4]],
            vec![vec![19.4, 19.9, 19.6], vec![21.2, 21.7, 21.5], vec![18.8, 19.3, 19.0], vec![20.5, 21.0, 20.8]],
        ],
        paired_rows: vec![[15.2, 21.4], [17.1, 23.0], [13.8, 19.6], [19.0, 25.2], [16.4, 22.1], [14.5, 20.4], [18.2, 24.3], [15.9, 21.8]],
        rm_timepoints: ["Week 0", "Week 4", "Week 8"],
        rm_rows: vec![
            vec![10.2, 10.8, 10.5, 11.1, 10.4, 10.9, 10.6, 11.0],
            vec![11.0, 11.4, 11.2, 11.7, 15.6, 16.2, 15.9, 16.5],
            vec![11.3, 11.8, 11.5, 12.0, 20.4, 21.1, 20.8, 21.5],
        ],
    }
}

/// Construct clean-room trial hazard-ratio limits dataset (`WF-63`).
#[must_use]
pub fn wf63_meta_analysis_forest_grouped() -> Wf63Dataset {
    Wf63Dataset {
        title: "Trial Forest WF-63",
        study_titles: vec![
            "Trial Alpha (2022)",
            "Trial Bravo (2023)",
            "Trial Cedar (2024)",
            "Trial Delta (2024)",
            "Trial Ember (2025)",
            "Trial Frost (2026)",
        ],
        entries: vec![[0.74, 0.56, 0.98], [0.88, 0.69, 1.12], [0.65, 0.48, 0.89], [0.95, 0.78, 1.16], [0.79, 0.61, 1.02], [0.71, 0.53, 0.95]],
    }
}

/// Construct clean-room signed two-cohort population pyramid dataset (`WF-64`).
#[must_use]
pub fn wf64_demographics_pyramid_grouped() -> Wf64Dataset {
    Wf64Dataset {
        title: "Cohort Pyramid WF-64",
        band_titles: vec!["0-14", "15-29", "30-44", "45-59", "60-74", "75-84", "85+"],
        cohort_titles: ["Cohort West", "Cohort East"],
        rows: vec![[-14.0, 13.5], [-21.0, 22.0], [-26.5, 25.0], [-23.0, 24.5], [-15.5, 16.0], [-7.5, 8.2], [-3.2, 3.8]],
    }
}

/// Construct clean-room longitudinal spaghetti XY trajectories (`WF-65`).
#[must_use]
pub fn wf65_longitudinal_spaghetti_xy() -> Wf65Dataset {
    Wf65Dataset {
        title: "Spaghetti Trajectories WF-65",
        weeks: vec![0.0, 2.0, 4.0, 8.0, 12.0],
        cohort_titles: ["Standard Care", "Intervention"],
        replicate_trajectories: vec![
            [vec![20.0, 22.5, 19.0, 21.2], vec![20.5, 21.8, 19.4, 22.0]],
            [vec![21.5, 23.8, 20.1, 22.0], vec![25.2, 27.1, 24.8, 26.4]],
            [vec![22.1, 24.4, 21.0, 22.9], vec![31.0, 33.6, 29.8, 32.5]],
            [vec![23.0, 25.1, 21.8, 23.6], vec![39.4, 41.8, 37.9, 40.2]],
            [vec![23.6, 25.9, 22.4, 24.1], vec![45.1, 48.0, 43.2, 46.7]],
        ],
        subject_titles: vec!["Subject S01", "Subject S02", "Subject S03", "Subject S04"],
        single_y_subjects: vec![
            vec![20.0, 21.0, 19.5, 22.0],
            vec![24.0, 26.5, 23.0, 27.5],
            vec![29.5, 32.0, 27.5, 34.0],
            vec![35.0, 39.0, 33.0, 40.5],
            vec![41.0, 44.5, 38.0, 46.0],
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn r13_shard8_synthetic_datasets_have_expected_finite_dimensions() {
        let d51 = wf51_inhibitor_triplicate_xy();
        assert_eq!(d51.kind, TableKind::Xy);
        assert_eq!(d51.subcolumns, SubcolumnMode::RawReplicates(3));
        assert_eq!(d51.known_rows.len(), 8);
        assert_eq!(d51.unknown_rows.len(), 2);

        let d52 = wf52_sparse_agonist_4pl_xy();
        assert_eq!(d52.series[0].rows.len(), 5);
        assert_eq!(d52.series[1].rows.len(), 5);

        let d53 = wf53_three_arm_survival();
        assert_eq!(d53.rows.len(), 9);
        assert!(d53.rows.iter().all(|r| r.status[0].is_none_or(|c| c == 0)));

        let d54 = wf54_dual_axis_mean_sd_n_xy();
        assert_eq!(d54.x_hours.len(), d54.fluorescence.len());
        assert_eq!(d54.x_hours.len(), d54.metabolite.len());

        let d55 = wf55_multisite_grouped_cohort();
        assert_eq!(d55.row_titles.len(), 12);
        assert_eq!(d55.col_titles.len(), 4);

        let d56 = wf56_exponential_growth_xy();
        assert_eq!(d56.rows.len(), 10);
        assert!(d56.rows.iter().all(|&(x, y)| x.is_finite() && y.is_finite()));

        let d57 = wf57_schematic_rgba();
        assert_eq!(d57.rgba.len(), (d57.width_px * d57.height_px * 4) as usize);

        let d58 = wf58_descriptive_column();
        assert_eq!(d58.rows.len(), 6);
    }

    #[test]
    fn r13_shard9_synthetic_datasets_have_expected_finite_dimensions() {
        let d59 = wf59_paired_before_after_and_inset();
        assert_eq!(d59.paired_rows.len(), 8);
        assert_eq!(d59.inset_xy.len(), 6);

        let d60 = wf60_subset_linear_with_outliers_xy();
        assert_eq!(d60.rows.len(), 8);

        let d61 = wf61_multivariate_biomarker_grouped();
        assert_eq!(d61.analyte_titles.len(), 13);
        assert_eq!(d61.rows.len(), 6);

        let d62 = wf62_five_design_trial();
        assert_eq!(d62.nested_values.len(), 2);
        assert_eq!(d62.nested_values[0].len(), 4);
        assert_eq!(d62.paired_rows.len(), 8);
        assert_eq!(d62.rm_rows.len(), 3);

        let d63 = wf63_meta_analysis_forest_grouped();
        assert_eq!(d63.entries.len(), 6);

        let d64 = wf64_demographics_pyramid_grouped();
        assert_eq!(d64.rows.len(), 7);
        assert!(d64.rows.iter().all(|r| r[0] < 0.0 && r[1] > 0.0));

        let d65 = wf65_longitudinal_spaghetti_xy();
        assert_eq!(d65.weeks.len(), 5);
        assert_eq!(d65.replicate_trajectories.len(), 5);
        assert_eq!(d65.single_y_subjects.len(), 5);
    }
}
