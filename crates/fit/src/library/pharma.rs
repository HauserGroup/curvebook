//! Pharmacology, Dose–Response, Receptor Binding, and Enzyme Kinetics built-in equations.
//!
//! Includes analytical gradients, smart data-driven initial parameter estimates, default
//! domain bounds, and derived scalar transformations (`EC50`, `IC50`, `Span`, `Ki`, `Km`).
//!
//! Every equation family cites the published source of its functional form in short
//! `Author (Year)` form on its model struct; the full references are collected here. Where the
//! library's parameterisation differs from the original paper (e.g. `log10` concentration on X,
//! `Bottom` / `Top` / `LogEC50` / `HillSlope`), the struct comment names the textbook that uses the
//! library's parameterisation (usually Motulsky & Christopoulos 2004).
//!
//! References:
//! - Hill, A. V. (1910). The possible effects of the aggregation of the molecules of hæmoglobin on
//!   its dissociation curves. *Journal of Physiology* 40:iv–vii.
//! - Motulsky, H. J., & Christopoulos, A. (2004). *Fitting Models to Biological Data Using Linear
//!   and Nonlinear Regression: A Practical Guide to Curve Fitting*. New York: Oxford University Press.
//! - Richards, F. J. (1959). A flexible growth function for empirical use. *Journal of
//!   Experimental Botany* 10. doi:10.1093/jxb/10.2.290.
//! - Gottschalk, P. G., & Dunn, J. R. (2005). The five-parameter logistic: a characterization and
//!   comparison with the four-parameter logistic. *Analytical Biochemistry* 343:54–65.
//!   doi:10.1016/j.ab.2005.04.035.
//! - Sebaugh, J. L. (2011). Guidelines for accurate EC50/IC50 estimation. *Pharmaceutical
//!   Statistics* 10:128–134. doi:10.1002/pst.426.
//! - Arunlakshana, O., & Schild, H. O. (1959). Some quantitative uses of drug antagonists.
//!   *British Journal of Pharmacology and Chemotherapy* 14:48–58.
//! - Lew, M. J., & Angus, J. A. (1995). Analysis of competitive agonist–antagonist interactions by
//!   nonlinear regression. *Trends in Pharmacological Sciences* 16:328–337.
//! - Ehlert, F. J. (1988). Estimation of the affinities of allosteric ligands using radioligand
//!   binding and pharmacological null methods. *Molecular Pharmacology* 33:187–194.
//! - Black, J. W., & Leff, P. (1983). Operational models of pharmacological agonism. *Proceedings
//!   of the Royal Society of London, Series B* 220:141–162.
//! - Black, J. W., Leff, P., Shankley, N. P., & Wood, J. (1985). An operational model of
//!   pharmacological agonism: the effect of E/\[A\] curve shape on agonist dissociation constant
//!   estimation. *British Journal of Pharmacology* 84:561–571. doi:10.1111/j.1476-5381.1985.tb12941.x.
//! - Langmuir, I. (1918). The adsorption of gases on plane surfaces of glass, mica and platinum.
//!   *Journal of the American Chemical Society* 40:1361–1403.
//! - Hulme, E. C., & Trevethick, M. A. (2010). Ligand binding assays at equilibrium: validation and
//!   interpretation. *British Journal of Pharmacology* 161:1219–1237.
//!   doi:10.1111/j.1476-5381.2009.00604.x.
//! - Cheng, Y., & Prusoff, W. H. (1973). Relationship between the inhibition constant (KI) and the
//!   concentration of inhibitor which causes 50 per cent inhibition (I50) of an enzymatic reaction.
//!   *Biochemical Pharmacology* 22:3099–3108. doi:10.1016/0006-2952(73)90196-2.
//! - DeBlasi, A., O'Reilly, K., & Motulsky, H. J. (1989). Calculating receptor number from binding
//!   experiments using same compound as radioligand and competitor. *Trends in Pharmacological
//!   Sciences* 10:227–229.
//! - Swillens, S. (1995). Interpretation of binding curves obtained with high receptor
//!   concentrations: practical aid for computer analysis. *Molecular Pharmacology* 47:1197–1203.
//! - Wang, Z.-X. (1995). An exact mathematical expression for describing competitive binding of two
//!   different ligands to a protein molecule. *FEBS Letters* 360:111–114.
//! - Motulsky, H. J., & Mahan, L. C. (1984). The kinetics of competitive radioligand binding
//!   predicted by the law of mass action. *Molecular Pharmacology* 25:1–9.
//! - Michaelis, L., & Menten, M. L. (1913). Die Kinetik der Invertinwirkung. *Biochemische
//!   Zeitschrift* 49:333–369.
//! - Haldane, J. B. S. (1930). *Enzymes*. London: Longmans, Green.
//! - Segel, I. H. (1975). *Enzyme Kinetics: Behavior and Analysis of Rapid Equilibrium and
//!   Steady-State Enzyme Systems*. New York: Wiley.
//! - Morrison, J. F. (1969). Kinetics of the reversible inhibition of enzyme-catalysed reactions by
//!   tight-binding inhibitors. *Biochimica et Biophysica Acta* 185:269–286.

use crate::library::{
    BuiltinEquationSpec, ModelMeta, ParametricModel, data_x_min_max, data_y_min_max, estimate_x_at_ymid, resolve_from_specs, safe_exp, spec_ids,
};
use crate::types::{DataPoint, DerivedScalar, EquationCategory};
use curvebook_doc::{ParameterRule, ParameterScope};

/// Ordered single-source-of-truth registrations for the pharmacology / biochemistry equation family.
pub(crate) const PHARMA_SPECS: &[BuiltinEquationSpec] = &[
    // Dose-Response Stimulation
    BuiltinEquationSpec::new("log_agonist_vs_response_3pl", &["log_agonist_vs_response", "ec50_3pl"], || {
        Box::new(LogDoseResponse {
            meta: ModelMeta::new(
                "log_agonist_vs_response_3pl",
                "log(agonist) vs. response (three parameters)",
                EquationCategory::DoseResponseStimulation,
                &["Bottom", "Top", "LogEC50"],
            ),
            variable_slope: false,
            is_inhibition: false,
            normalized: false,
        })
    }),
    BuiltinEquationSpec::new("log_agonist_vs_response_4pl", &["log_agonist_vs_response_variable_slope", "4pl"], || {
        Box::new(LogDoseResponse {
            meta: ModelMeta::new(
                "log_agonist_vs_response_4pl",
                "log(agonist) vs. response, variable Hill slope (four parameters)",
                EquationCategory::DoseResponseStimulation,
                &["Bottom", "Top", "LogEC50", "HillSlope"],
            ),
            variable_slope: true,
            is_inhibition: false,
            normalized: false,
        })
    }),
    BuiltinEquationSpec::new("log_agonist_vs_response_normalized", &[], || {
        Box::new(LogDoseResponse {
            meta: ModelMeta::new(
                "log_agonist_vs_response_normalized",
                "log(agonist) vs. normalized response",
                EquationCategory::DoseResponseStimulation,
                &["LogEC50"],
            ),
            variable_slope: false,
            is_inhibition: false,
            normalized: true,
        })
    }),
    BuiltinEquationSpec::new("log_agonist_vs_response_normalized_var", &[], || {
        Box::new(LogDoseResponse {
            meta: ModelMeta::new(
                "log_agonist_vs_response_normalized_var",
                "log(agonist) vs. normalized response, variable Hill slope",
                EquationCategory::DoseResponseStimulation,
                &["LogEC50", "HillSlope"],
            ),
            variable_slope: true,
            is_inhibition: false,
            normalized: true,
        })
    }),
    BuiltinEquationSpec::new("agonist_vs_response_3pl", &["agonist_vs_response"], || {
        Box::new(LinearDoseResponse {
            meta: ModelMeta::new(
                "agonist_vs_response_3pl",
                "[Agonist] vs. response (three parameters)",
                EquationCategory::DoseResponseStimulation,
                &["Bottom", "Top", "EC50"],
            ),
            variable_slope: false,
            is_inhibition: false,
        })
    }),
    BuiltinEquationSpec::new("agonist_vs_response_4pl", &[], || {
        Box::new(LinearDoseResponse {
            meta: ModelMeta::new(
                "agonist_vs_response_4pl",
                "[Agonist] vs. response, variable Hill slope (four parameters)",
                EquationCategory::DoseResponseStimulation,
                &["Bottom", "Top", "EC50", "HillSlope"],
            ),
            variable_slope: true,
            is_inhibition: false,
        })
    }),
    BuiltinEquationSpec::new("log_agonist_vs_response_5pl", &["5pl"], || Box::new(RichardsFivePl)),
    BuiltinEquationSpec::new("bell_shaped_dose_response", &[], || Box::new(BellShapedDoseResponse)),
    BuiltinEquationSpec::new("biphasic_dose_response", &[], || Box::new(BiphasicDoseResponse)),
    BuiltinEquationSpec::new("log_agonist_vs_response_ecf", &["ecf"], || Box::new(DoseResponseEcF)),
    BuiltinEquationSpec::new("ec50_shift_gaddum_schild", &["gaddum_schild"], || Box::new(GaddumSchild)),
    BuiltinEquationSpec::new("allosteric_ec50_shift", &[], || Box::new(AllostericEc50Shift)),
    BuiltinEquationSpec::new("operational_model_agonist", &[], || Box::new(OperationalAgonism)),
    // Dose-Response Inhibition
    BuiltinEquationSpec::new("log_inhibitor_vs_response_3pl", &["log_inhibitor_vs_response", "ic50_3pl"], || {
        Box::new(LogDoseResponse {
            meta: ModelMeta::new(
                "log_inhibitor_vs_response_3pl",
                "log(inhibitor) vs. response (three parameters)",
                EquationCategory::DoseResponseInhibition,
                &["Bottom", "Top", "LogIC50"],
            ),
            variable_slope: false,
            is_inhibition: true,
            normalized: false,
        })
    }),
    BuiltinEquationSpec::new("log_inhibitor_vs_response_4pl", &["log_inhibitor_vs_response_variable_slope"], || {
        Box::new(LogDoseResponse {
            meta: ModelMeta::new(
                "log_inhibitor_vs_response_4pl",
                "log(inhibitor) vs. response, variable Hill slope (four parameters)",
                EquationCategory::DoseResponseInhibition,
                &["Bottom", "Top", "LogIC50", "HillSlope"],
            ),
            variable_slope: true,
            is_inhibition: true,
            normalized: false,
        })
    }),
    BuiltinEquationSpec::new("log_inhibitor_vs_response_normalized", &[], || {
        Box::new(LogDoseResponse {
            meta: ModelMeta::new(
                "log_inhibitor_vs_response_normalized",
                "log(inhibitor) vs. normalized response",
                EquationCategory::DoseResponseInhibition,
                &["LogIC50"],
            ),
            variable_slope: false,
            is_inhibition: true,
            normalized: true,
        })
    }),
    BuiltinEquationSpec::new("log_inhibitor_vs_response_normalized_var", &[], || {
        Box::new(LogDoseResponse {
            meta: ModelMeta::new(
                "log_inhibitor_vs_response_normalized_var",
                "log(inhibitor) vs. normalized response, variable Hill slope",
                EquationCategory::DoseResponseInhibition,
                &["LogIC50", "HillSlope"],
            ),
            variable_slope: true,
            is_inhibition: true,
            normalized: true,
        })
    }),
    BuiltinEquationSpec::new("inhibitor_vs_response_3pl", &["inhibitor_vs_response"], || {
        Box::new(LinearDoseResponse {
            meta: ModelMeta::new(
                "inhibitor_vs_response_3pl",
                "[Inhibitor] vs. response (three parameters)",
                EquationCategory::DoseResponseInhibition,
                &["Bottom", "Top", "IC50"],
            ),
            variable_slope: false,
            is_inhibition: true,
        })
    }),
    BuiltinEquationSpec::new("inhibitor_vs_response_4pl", &[], || {
        Box::new(LinearDoseResponse {
            meta: ModelMeta::new(
                "inhibitor_vs_response_4pl",
                "[Inhibitor] vs. response, variable Hill slope (four parameters)",
                EquationCategory::DoseResponseInhibition,
                &["Bottom", "Top", "IC50", "HillSlope"],
            ),
            variable_slope: true,
            is_inhibition: true,
        })
    }),
    BuiltinEquationSpec::new("absolute_ic50_4pl", &[], || Box::new(AbsoluteIc50)),
    // Receptor Binding
    BuiltinEquationSpec::new("one_site_specific_binding", &[], || {
        Box::new(OneSiteBinding {
            meta: ModelMeta::new("one_site_specific_binding", "One-site specific binding", EquationCategory::ReceptorBinding, &["Bmax", "Kd"]),
            with_hill: false,
            with_ns: false,
        })
    }),
    BuiltinEquationSpec::new("one_site_specific_hill", &[], || {
        Box::new(OneSiteBinding {
            meta: ModelMeta::new(
                "one_site_specific_hill",
                "One-site specific binding with Hill slope",
                EquationCategory::ReceptorBinding,
                &["Bmax", "Kd", "h"],
            ),
            with_hill: true,
            with_ns: false,
        })
    }),
    BuiltinEquationSpec::new("one_site_total_and_ns", &[], || {
        Box::new(OneSiteBinding {
            meta: ModelMeta::new(
                "one_site_total_and_ns",
                "One-site total and nonspecific binding",
                EquationCategory::ReceptorBinding,
                &["Bmax", "Kd", "NS", "Background"],
            ),
            with_hill: false,
            with_ns: true,
        })
    }),
    BuiltinEquationSpec::new("one_site_total_and_ns_global", &[], || Box::new(OneSiteTotalAndNsGlobal)),
    BuiltinEquationSpec::new("two_site_total_and_ns_global", &[], || Box::new(TwoSiteTotalAndNsGlobal)),
    BuiltinEquationSpec::new("one_site_specific_ligand_depletion", &[], || Box::new(OneSiteSpecificLigandDepletion)),
    BuiltinEquationSpec::new("one_site_competition_ligand_depletion", &[], || Box::new(OneSiteCompetitionLigandDepletion)),
    BuiltinEquationSpec::new("allosteric_modulator_saturation", &[], || Box::new(AllostericModulatorSaturation)),
    BuiltinEquationSpec::new("homologous_one_site_competition", &[], || Box::new(HomologousOneSiteCompetition)),
    BuiltinEquationSpec::new("two_sites_specific_binding", &[], || Box::new(TwoSiteBinding)),
    BuiltinEquationSpec::new("one_site_fit_ki", &[], || Box::new(CompetitionOneSiteKi)),
    BuiltinEquationSpec::new("one_site_fit_logic50", &[], || {
        Box::new(LogDoseResponse {
            meta: ModelMeta::new(
                "one_site_fit_logic50",
                "One-site competitive binding (logIC50)",
                EquationCategory::ReceptorBinding,
                &["Bottom", "Top", "LogIC50"],
            ),
            variable_slope: false,
            is_inhibition: true,
            normalized: false,
        })
    }),
    BuiltinEquationSpec::new("two_sites_fit_ki", &[], || Box::new(CompetitionTwoSiteKi)),
    BuiltinEquationSpec::new("dissociation_one_phase", &[], || Box::new(DissociationOnePhase)),
    BuiltinEquationSpec::new("association_kinetics_one_conc", &[], || Box::new(AssociationOneConc)),
    BuiltinEquationSpec::new("association_kinetics_two_conc", &[], || Box::new(AssociationTwoConc)),
    BuiltinEquationSpec::new("assoc_then_dissoc", &[], || Box::new(AssocThenDissoc)),
    BuiltinEquationSpec::new("motulsky_mahan_kinetics", &[], || Box::new(MotulskyMahanKinetics)),
    // Enzyme Kinetics
    BuiltinEquationSpec::new("michaelis_menten", &[], || {
        Box::new(MichaelisMenten {
            meta: ModelMeta::new("michaelis_menten", "Michaelis-Menten", EquationCategory::EnzymeKinetics, &["Vmax", "Km"]),
            kcat_mode: false,
        })
    }),
    BuiltinEquationSpec::new("kcat_enzyme_kinetics", &["kcat"], || {
        Box::new(MichaelisMenten {
            meta: ModelMeta::new("kcat_enzyme_kinetics", "kcat", EquationCategory::EnzymeKinetics, &["kcat", "Km", "Et"]),
            kcat_mode: true,
        })
    }),
    BuiltinEquationSpec::new("allosteric_sigmoidal_kinetics", &["allosteric_sigmoidal"], || Box::new(AllostericSigmoidal)),
    BuiltinEquationSpec::new("substrate_inhibition", &[], || Box::new(SubstrateInhibition)),
    BuiltinEquationSpec::new("competitive_inhibition", &[], || {
        Box::new(EnzymeInhibition {
            meta: ModelMeta::new("competitive_inhibition", "Competitive inhibition", EquationCategory::EnzymeKinetics, &["Vmax", "Km", "Ki", "I"]),
            mode: EnzymeInhibMode::Competitive,
        })
    }),
    BuiltinEquationSpec::new("noncompetitive_inhibition", &[], || {
        Box::new(EnzymeInhibition {
            meta: ModelMeta::new(
                "noncompetitive_inhibition",
                "Noncompetitive inhibition",
                EquationCategory::EnzymeKinetics,
                &["Vmax", "Km", "Ki", "I"],
            ),
            mode: EnzymeInhibMode::Noncompetitive,
        })
    }),
    BuiltinEquationSpec::new("uncompetitive_inhibition", &[], || {
        Box::new(EnzymeInhibition {
            meta: ModelMeta::new(
                "uncompetitive_inhibition",
                "Uncompetitive inhibition",
                EquationCategory::EnzymeKinetics,
                &["Vmax", "Km", "alphaKi", "I"],
            ),
            mode: EnzymeInhibMode::Uncompetitive,
        })
    }),
    BuiltinEquationSpec::new("mixed_model_inhibition", &[], || {
        Box::new(EnzymeInhibition {
            meta: ModelMeta::new(
                "mixed_model_inhibition",
                "Mixed model inhibition",
                EquationCategory::EnzymeKinetics,
                &["Vmax", "Km", "Ki", "Alpha", "I"],
            ),
            mode: EnzymeInhibMode::Mixed,
        })
    }),
    BuiltinEquationSpec::new("morrison_tight_binding_ki", &["morrison"], || Box::new(MorrisonKi)),
];

const PHARMA_EQUATION_ID_ARRAY: [&str; PHARMA_SPECS.len()] = spec_ids::<{ PHARMA_SPECS.len() }>(PHARMA_SPECS);

/// Canonical IDs exposed by the pharmacology / biochemistry equation catalog.
pub const PHARMA_EQUATION_IDS: &[&str] = &PHARMA_EQUATION_ID_ARRAY;

#[inline]
fn safe_pow10(exp: f64) -> Option<f64> {
    if !exp.is_finite() || !(-300.0..=300.0).contains(&exp) {
        return None;
    }
    let v = 10.0_f64.powf(exp);
    if v.is_finite() { Some(v) } else { None }
}

#[inline]
fn sigmoid_from_log_x(x: f64, log_c50: f64, slope: f64) -> Option<(f64, f64)> {
    let arg = (log_c50 - x) * slope;
    if !arg.is_finite() {
        return None;
    }
    let exp_ln10 = arg * core::f64::consts::LN_10;
    if exp_ln10 > 700.0 {
        return Some((0.0, f64::INFINITY));
    }
    if exp_ln10 < -700.0 {
        return Some((1.0, 0.0));
    }
    let p = 10.0_f64.powf(arg);
    let sig = 1.0 / (1.0 + p);
    if sig.is_finite() { Some((sig, p)) } else { None }
}

/// Resolve any pharmacology equation ID (with friendly aliases) into a [`ParametricModel`].
#[must_use]
pub fn resolve_pharma_model(norm_id: &str) -> Option<Box<dyn ParametricModel>> {
    resolve_from_specs(PHARMA_SPECS, norm_id)
}

/// Three/four-parameter logistic on log10 concentration:
/// `Y = Bottom + (Top - Bottom) / (1 + 10^((LogEC50 - X) * HillSlope))` (agonist; the inhibitor
/// form uses `(X - LogIC50) * HillSlope`). The normalized variants fix `Bottom = 0`, `Top = 100`.
/// Reference: Hill (1910) equation; this log-X `Bottom` / `Top` / `LogEC50` / `HillSlope`
/// parameterisation follows Motulsky & Christopoulos (2004).
struct LogDoseResponse {
    meta: ModelMeta,
    variable_slope: bool,
    is_inhibition: bool,
    normalized: bool,
}

impl ParametricModel for LogDoseResponse {
    impl_meta_methods!();
    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let x0 = x.first().copied()?;
        let (bottom, top, log_c50, hill) = match (self.normalized, self.variable_slope) {
            (true, false) => (0.0, 100.0, params[0], 1.0),
            (true, true) => (0.0, 100.0, params[0], params[1]),
            (false, false) => (params[0], params[1], params[2], 1.0),
            (false, true) => (params[0], params[1], params[2], params[3]),
        };
        let eff_slope = if self.is_inhibition { -hill } else { hill };
        let (sig, _) = sigmoid_from_log_x(x0, log_c50, eff_slope)?;
        Some(bottom + (top - bottom) * sig)
    }
    fn grad(&self, x: &[f64], params: &[f64], _dataset_index: usize, out: &mut [f64]) -> bool {
        let Some(&x0) = x.first() else {
            return false;
        };
        let (bottom, top, log_c50, hill) = match (self.normalized, self.variable_slope) {
            (true, false) => (0.0, 100.0, params[0], 1.0),
            (true, true) => (0.0, 100.0, params[0], params[1]),
            (false, false) => (params[0], params[1], params[2], 1.0),
            (false, true) => (params[0], params[1], params[2], params[3]),
        };
        let eff_slope = if self.is_inhibition { -hill } else { hill };
        let Some((sig, p)) = sigmoid_from_log_x(x0, log_c50, eff_slope) else {
            return false;
        };
        let sig_deriv = if !p.is_finite() {
            0.0
        } else if p <= 1.0 {
            p * sig * sig
        } else {
            let inv = 1.0 / p;
            (inv / (1.0 + inv)) / (1.0 + inv)
        };
        let span = top - bottom;
        let common = -span * sig_deriv * core::f64::consts::LN_10;
        let d_log_c50 = common * eff_slope;
        let d_hill = common * (log_c50 - x0) * if self.is_inhibition { -1.0 } else { 1.0 };
        match (self.normalized, self.variable_slope) {
            (true, false) => {
                out[0] = d_log_c50;
            }
            (true, true) => {
                out[0] = d_log_c50;
                out[1] = d_hill;
            }
            (false, false) => {
                out[0] = 1.0 - sig;
                out[1] = sig;
                out[2] = d_log_c50;
            }
            (false, true) => {
                out[0] = 1.0 - sig;
                out[1] = sig;
                out[2] = d_log_c50;
                out[3] = d_hill;
            }
        }
        out.iter().all(|v| v.is_finite())
    }
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (ymin, ymax) = data_y_min_max(points);
        let log_mid = estimate_x_at_ymid(points);
        match (self.normalized, self.variable_slope) {
            (true, false) => vec![log_mid],
            (true, true) => vec![log_mid, 1.0],
            (false, false) => vec![ymin, ymax, log_mid],
            (false, true) => vec![ymin, ymax, log_mid, 1.0],
        }
    }
    fn derived_transforms(&self, params: &[f64]) -> Vec<DerivedScalar> {
        let mut out = Vec::new();
        let (span, log_c50) = match (self.normalized, self.variable_slope) {
            (true, _) => (100.0, params[0]),
            (false, _) => (params[1] - params[0], params[2]),
        };
        let label = if self.is_inhibition { "IC50" } else { "EC50" };
        if let Some(c50) = safe_pow10(log_c50) {
            out.push(DerivedScalar { name: label.to_owned(), value: c50, units: None });
        }
        if !self.normalized {
            out.push(DerivedScalar { name: "Span".to_owned(), value: span, units: None });
        }
        out
    }
}

/// Hill equation on linear concentration:
/// `Y = Bottom + (Top - Bottom) * X^h / (EC50^h + X^h)` (inhibitor form uses `EC50^h / (EC50^h + X^h)`).
/// Reference: Hill (1910); parameterisation per Motulsky & Christopoulos (2004).
struct LinearDoseResponse {
    meta: ModelMeta,
    variable_slope: bool,
    is_inhibition: bool,
}

impl ParametricModel for LinearDoseResponse {
    impl_meta_methods!();
    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let c = x.first().copied()?;
        if c < 0.0 || !c.is_finite() {
            return None;
        }
        let bottom = params[0];
        let top = params[1];
        let c50 = params[2];
        let hill = if self.variable_slope { params[3] } else { 1.0 };
        if c50 <= 0.0 {
            return None;
        }
        if c == 0.0 {
            return Some(if self.is_inhibition { top } else { bottom });
        }
        let ratio = (c / c50).powf(hill);
        if !ratio.is_finite() {
            return Some(if self.is_inhibition { bottom } else { top });
        }
        let frac = if self.is_inhibition { 1.0 / (1.0 + ratio) } else { ratio / (1.0 + ratio) };
        Some(bottom + (top - bottom) * frac)
    }

    fn grad(&self, x: &[f64], params: &[f64], _dataset_index: usize, out: &mut [f64]) -> bool {
        let Some(&c) = x.first() else {
            return false;
        };
        if !c.is_finite() || c < 0.0 || params.len() < self.meta.params.len() || out.len() < self.meta.params.len() {
            return false;
        }
        let bottom = params[0];
        let top = params[1];
        let c50 = params[2];
        let hill = if self.variable_slope { params[3] } else { 1.0 };
        if !bottom.is_finite() || !top.is_finite() || !c50.is_finite() || c50 <= 0.0 || !hill.is_finite() {
            return false;
        }
        if c == 0.0 {
            out[0] = if self.is_inhibition { 0.0 } else { 1.0 };
            out[1] = if self.is_inhibition { 1.0 } else { 0.0 };
            out[2] = 0.0;
            if self.variable_slope {
                out[3] = 0.0;
            }
            return true;
        }
        let ratio = (c / c50).powf(hill);
        if !ratio.is_finite() {
            out[0] = if self.is_inhibition { 1.0 } else { 0.0 };
            out[1] = if self.is_inhibition { 0.0 } else { 1.0 };
            out[2] = 0.0;
            if self.variable_slope {
                out[3] = 0.0;
            }
            return true;
        }
        let frac = if self.is_inhibition { 1.0 / (1.0 + ratio) } else { ratio / (1.0 + ratio) };
        let du_d_ln = if ratio <= 1.0 {
            (ratio / (1.0 + ratio)) / (1.0 + ratio)
        } else {
            let inv = 1.0 / ratio;
            (inv / (1.0 + inv)) / (1.0 + inv)
        };
        let span = top - bottom;
        let sign = if self.is_inhibition { -1.0 } else { 1.0 };
        out[0] = 1.0 - frac;
        out[1] = frac;
        out[2] = span * sign * du_d_ln * (-hill / c50);
        if self.variable_slope {
            out[3] = span * sign * du_d_ln * (c / c50).ln();
        }
        out[..self.meta.params.len()].iter().all(|v| v.is_finite())
    }
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (ymin, ymax) = data_y_min_max(points);
        let c50 = estimate_x_at_ymid(points).max(1e-6);
        if self.variable_slope { vec![ymin, ymax, c50, 1.0] } else { vec![ymin, ymax, c50] }
    }
    fn default_bounds(&self) -> Vec<(Option<f64>, Option<f64>)> {
        if self.variable_slope {
            vec![(None, None), (None, None), (Some(1e-30), None), (Some(0.05), Some(20.0))]
        } else {
            vec![(None, None), (None, None), (Some(1e-30), None)]
        }
    }
}

/// Five-parameter (asymmetric) logistic:
/// `Y = Bottom + (Top - Bottom) / (1 + 10^((LogXb - X) * HillSlope))^S`; `S = 1` gives the 4PL.
/// Derived `LogEC50 = LogXb - log10(2^(1/S) - 1) / HillSlope` (the X giving half the span).
/// Reference: Richards (1959) generalised logistic; the asymmetry-exponent form used for
/// dose-response is that of Gottschalk & Dunn (2005), here with log10 X.
struct RichardsFivePl;

impl ParametricModel for RichardsFivePl {
    impl_static_meta!(
        "log_agonist_vs_response_5pl",
        "Asymmetric (five parameters) log(agonist) vs. response",
        EquationCategory::DoseResponseStimulation,
        ["Bottom", "Top", "LogXb", "HillSlope", "S"]
    );
    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let x0 = x.first().copied()?;
        let (bottom, top, log_xb, hill, s) = (params[0], params[1], params[2], params[3], params[4]);
        if s <= 0.0 {
            return None;
        }
        let (sig, _) = sigmoid_from_log_x(x0, log_xb, hill)?;
        let asym = sig.powf(s);
        if asym.is_finite() { Some(bottom + (top - bottom) * asym) } else { None }
    }
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (ymin, ymax) = data_y_min_max(points);
        vec![ymin, ymax, estimate_x_at_ymid(points), 1.0, 1.0]
    }
    fn default_bounds(&self) -> Vec<(Option<f64>, Option<f64>)> {
        vec![(None, None), (None, None), (None, None), (None, None), (Some(1e-4), Some(100.0))]
    }
    fn derived_transforms(&self, params: &[f64]) -> Vec<DerivedScalar> {
        let (log_xb, hill, s) = (params[2], params[3], params[4]);
        let mut out = Vec::new();
        if log_xb.is_finite() && hill.is_finite() && s.is_finite() && s > 0.0 && hill.abs() > 1e-12 {
            let log10_term = if s == 1.0 {
                0.0
            } else if s < 0.01 {
                let ln_term = (-core::f64::consts::LN_2 / s).exp();
                core::f64::consts::LOG10_2 / s + (-ln_term).ln_1p() / core::f64::consts::LN_10
            } else if s > 1e100 {
                core::f64::consts::LN_2.log10() - s.log10()
            } else {
                (core::f64::consts::LN_2 / s).exp_m1().log10()
            };
            let log_ec50 = log_xb - log10_term / hill;
            if log_ec50.is_finite() {
                out.push(DerivedScalar { name: "LogEC50".to_owned(), value: log_ec50, units: None });
                if let Some(ec50) = safe_pow10(log_ec50) {
                    out.push(DerivedScalar { name: "EC50".to_owned(), value: ec50, units: None });
                }
            }
        }
        out
    }
}

/// Bell-shaped dose-response: a rising and a falling Hill term sharing a `Dip` level,
/// `Y = Dip + (Plateau1 - Dip) * s1 + (Plateau2 - Dip) * (1 - s2)`, `si = 1 / (1 + 10^((X - LogEC50_i) * nHi))`.
/// Reference: built from two Hill (1910) log-logistic terms; no single primary source for this
/// composite.
struct BellShapedDoseResponse;

impl ParametricModel for BellShapedDoseResponse {
    impl_static_meta!(
        "bell_shaped_dose_response",
        "Bell-shaped dose-response",
        EquationCategory::DoseResponseStimulation,
        ["Plateau1", "Plateau2", "Dip", "LogEC50_1", "LogEC50_2", "nH1", "nH2"]
    );
    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let x0 = x.first().copied()?;
        let (p1, p2, dip, l1, l2, n1, n2) = (params[0], params[1], params[2], params[3], params[4], params[5], params[6]);
        let (s1, _) = sigmoid_from_log_x(x0, l1, -n1)?;
        let (s2, _) = sigmoid_from_log_x(x0, l2, -n2)?;
        Some(dip + (p1 - dip) * s1 + (p2 - dip) * (1.0 - s2))
    }
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (ymin, ymax) = data_y_min_max(points);
        let (xmin, xmax) = data_x_min_max(points);
        let span = (xmax - xmin).max(1.0);
        vec![ymin, ymin, ymax, xmin + 0.3 * span, xmin + 0.7 * span, 1.0, 1.0]
    }
}

/// Determine whether `(x, y)` exhibits a downward trend via OLS covariance sign.
fn estimate_curve_is_falling(points: &[DataPoint]) -> bool {
    let mut count = 0_usize;
    let mut sum_x = 0.0_f64;
    let mut sum_y = 0.0_f64;
    for p in points {
        let x = p.x0();
        if x.is_finite() && p.y.is_finite() {
            sum_x += x;
            sum_y += p.y;
            count += 1;
        }
    }
    if count < 2 {
        return false;
    }
    let n = count as f64;
    let mean_x = sum_x / n;
    let mean_y = sum_y / n;
    let mut cov = 0.0_f64;
    let mut var_x = 0.0_f64;
    for p in points {
        let x = p.x0();
        if x.is_finite() && p.y.is_finite() {
            let dx = x - mean_x;
            cov += dx * (p.y - mean_y);
            var_x += dx * dx;
        }
    }
    var_x > 0.0 && cov < 0.0
}

/// Biphasic dose-response: weighted sum (`Frac`, `1 - Frac`) of two log-logistic Hill terms with
/// separate `LogEC50` and Hill slopes, each term `1 / (1 + 10^((LogEC50_i - X) * nHi))`,
/// matching the library agonist slope convention (Motulsky & Christopoulos 2004; Hill 1910):
/// positive `nHi` gives a rising curve with `X`, negative `nHi` a falling curve.
/// Reference: built from two Hill (1910) log-logistic terms (two-site analogue); no single
/// primary source for this composite.
struct BiphasicDoseResponse;

impl ParametricModel for BiphasicDoseResponse {
    impl_static_meta!(
        "biphasic_dose_response",
        "Biphasic dose-response",
        EquationCategory::DoseResponseStimulation,
        ["Bottom", "Top", "Frac", "LogEC50_1", "LogEC50_2", "nH1", "nH2"]
    );
    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let x0 = x.first().copied()?;
        let (bottom, top, frac, l1, l2, n1, n2) = (params[0], params[1], params[2], params[3], params[4], params[5], params[6]);
        let (s1, _) = sigmoid_from_log_x(x0, l1, n1)?;
        let (s2, _) = sigmoid_from_log_x(x0, l2, n2)?;
        Some(bottom + (top - bottom) * (frac * s1 + (1.0 - frac) * s2))
    }
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (ymin, ymax) = data_y_min_max(points);
        let (xmin, xmax) = data_x_min_max(points);
        let span = (xmax - xmin).max(1.0);
        let slope = if estimate_curve_is_falling(points) { -1.0 } else { 1.0 };
        vec![ymin, ymax, 0.5, xmin + 0.3 * span, xmin + 0.7 * span, slope, slope]
    }
    fn default_bounds(&self) -> Vec<(Option<f64>, Option<f64>)> {
        vec![(None, None), (None, None), (Some(0.0), Some(1.0)), (None, None), (None, None), (None, None), (None, None)]
    }
}

/// 4PL reparameterised to fit `LogECF` (the concentration giving `F` % of the span) directly:
/// `LogEC50 = LogECF + log10((100 - F) / F) / HillSlope`.
/// Reference: algebraic reparameterisation of the Hill (1910) log-logistic in the
/// Motulsky & Christopoulos (2004) parameterisation.
struct DoseResponseEcF;

impl ParametricModel for DoseResponseEcF {
    impl_static_meta!(
        "log_agonist_vs_response_ecf",
        "ECanything (log(agonist) vs. response)",
        EquationCategory::DoseResponseStimulation,
        ["Bottom", "Top", "LogECF", "HillSlope", "F"]
    );
    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let x0 = x.first().copied()?;
        let (bottom, top, log_ecf, hill, f_pct) = (params[0], params[1], params[2], params[3], params[4]);
        if f_pct <= 0.0 || f_pct >= 100.0 || hill.abs() < 1e-12 {
            return None;
        }
        let log_ec50 = log_ecf + ((100.0 - f_pct) / f_pct).log10() / hill;
        let (sig, _) = sigmoid_from_log_x(x0, log_ec50, hill)?;
        Some(bottom + (top - bottom) * sig)
    }
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (ymin, ymax) = data_y_min_max(points);
        vec![ymin, ymax, estimate_x_at_ymid(points), 1.0, 50.0]
    }
}

/// 4PL inhibitor curve fitted for the absolute IC50 (concentration at which `Y` equals
/// `(Control0 + Control100) / 2`, i.e. 50 % of the response window defined by the 0 % and 100 %
/// assay controls):
/// `LogIC50 = LogAbsIC50 - log10((Top - Y50) / (Y50 - Bottom)) / HillSlope`,
/// where `Y50 = (Control0 + Control100) / 2`.
/// Reference: absolute vs relative IC50, Sebaugh (2011); curve form per Hill (1910).
struct AbsoluteIc50;

impl ParametricModel for AbsoluteIc50 {
    impl_static_meta!(
        "absolute_ic50_4pl",
        "Absolute IC50, four parameter",
        EquationCategory::DoseResponseInhibition,
        ["Bottom", "Top", "LogIC50", "HillSlope", "Control0", "Control100"]
    );
    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let x0 = x.first().copied()?;
        let (bottom, top, log_abs_ic50, hill, control0, control100) = (params[0], params[1], params[2], params[3], params[4], params[5]);
        let fifty = 0.5 * (control0 + control100);
        if !fifty.is_finite() || top <= fifty || bottom >= fifty || (top - bottom).abs() < 1e-15 || hill.abs() < 1e-12 {
            return None;
        }
        let log_ic50 = log_abs_ic50 - ((top - fifty) / (fifty - bottom)).log10() / hill;
        let (sig, _) = sigmoid_from_log_x(x0, log_ic50, -hill)?;
        Some(bottom + (top - bottom) * sig)
    }
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (ymin, ymax) = data_y_min_max(points);
        vec![ymin, ymax, estimate_x_at_ymid(points), 1.0, ymin, ymax]
    }
    fn default_parameter_rules(&self, points: &[DataPoint]) -> Vec<ParameterRule> {
        let (ymin, ymax) = data_y_min_max(points);
        vec![
            ParameterRule {
                name: "Control0".to_owned(),
                scope: ParameterScope::Constant(ymin),
                initial_override: None,
                lower_bound: None,
                upper_bound: None,
            },
            ParameterRule {
                name: "Control100".to_owned(),
                scope: ParameterScope::Constant(ymax),
                initial_override: None,
                lower_bound: None,
                upper_bound: None,
            },
        ]
    }
}

/// Agonist 4PL whose EC50 is shifted by a competitive antagonist at `AntagConc`:
/// `LogEC50' = LogEC50 + log10(1 + (AntagConc * 10^pA2)^SchildSlope)`.
/// Reference: Schild equation, Arunlakshana & Schild (1959); global nonlinear-regression form
/// per Lew & Angus (1995). `pA2` is `-log10` of the antagonist concentration giving a dose ratio
/// of 2; it equals `pKB` only when `SchildSlope = 1`.
struct GaddumSchild;

impl ParametricModel for GaddumSchild {
    impl_static_meta!(
        "ec50_shift_gaddum_schild",
        "Gaddum/Schild EC50 shift",
        EquationCategory::DoseResponseStimulation,
        ["Bottom", "Top", "LogEC50", "HillSlope", "pA2", "SchildSlope", "AntagConc"]
    );
    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let x0 = x.first().copied()?;
        let (bottom, top, log_ec50, hill, pa2, schild_slope, antag) = (params[0], params[1], params[2], params[3], params[4], params[5], params[6]);
        let p_kb = pa2;
        let log_dr = if antag > 0.0 { (1.0 + (antag * safe_pow10(p_kb)?).powf(schild_slope)).log10() } else { 0.0 };
        let (sig, _) = sigmoid_from_log_x(x0, log_ec50 + log_dr, hill)?;
        Some(bottom + (top - bottom) * sig)
    }
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (ymin, ymax) = data_y_min_max(points);
        vec![ymin, ymax, estimate_x_at_ymid(points), 1.0, 7.0, 1.0, 0.0]
    }
}

/// Agonist 4PL whose EC50 is shifted by an allosteric modulator (affinity cooperativity `alpha`,
/// modulator dissociation constant `Kb`): `EC50' = EC50 * (1 + B/Kb) / (1 + alpha * B/Kb)`.
/// Reference: allosteric ternary complex model, Ehlert (1988).
struct AllostericEc50Shift;

impl ParametricModel for AllostericEc50Shift {
    impl_static_meta!(
        "allosteric_ec50_shift",
        "Allosteric EC50 shift",
        EquationCategory::DoseResponseStimulation,
        ["Bottom", "Top", "LogEC50", "HillSlope", "Kb", "alpha", "ModConc"]
    );
    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let x0 = x.first().copied()?;
        let (bottom, top, log_ec50, hill, kb, alpha, mod_conc) = (params[0], params[1], params[2], params[3], params[4], params[5], params[6]);
        if kb <= 0.0 || alpha <= 0.0 || mod_conc < 0.0 {
            return None;
        }
        let ratio = (1.0 + mod_conc / kb) / (1.0 + alpha * mod_conc / kb);
        let shifted = log_ec50 + ratio.log10();
        let (sig, _) = sigmoid_from_log_x(x0, shifted, hill)?;
        Some(bottom + (top - bottom) * sig)
    }
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (ymin, ymax) = data_y_min_max(points);
        vec![ymin, ymax, estimate_x_at_ymid(points), 1.0, 1e-6, 1.0, 0.0]
    }
}

/// Operational model of agonism with transducer slope `n`, `X = log10[A]`:
/// `E = Basal + (Emax - Basal) * tau^n [A]^n / (([A] + KA)^n + tau^n [A]^n)`.
/// Reference: Black & Leff (1983); the slope-`n` form is Black, Leff, Shankley & Wood (1985).
/// `Emax` is the system maximum (`Em`), not the agonist's observed maximum.
struct OperationalAgonism;

impl ParametricModel for OperationalAgonism {
    impl_static_meta!(
        "operational_model_agonist",
        "Black–Leff operational model, partial agonist",
        EquationCategory::DoseResponseStimulation,
        ["Basal", "Emax", "LogKA", "LogTau", "n"]
    );
    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let x0 = x.first().copied()?;
        let (basal, emax, log_ka, log_tau, n) = (params[0], params[1], params[2], params[3], params[4]);
        let a = safe_pow10(x0)?;
        let ka = safe_pow10(log_ka)?;
        let tau = safe_pow10(log_tau)?;
        let num = tau * a;
        let den = a + ka;
        if den <= 0.0 {
            return None;
        }
        let operate = (num / den).powf(n);
        if !operate.is_finite() {
            return None;
        }
        let term = operate / (1.0 + operate);
        Some(basal + (emax - basal) * term)
    }
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (ymin, ymax) = data_y_min_max(points);
        vec![ymin, ymax, estimate_x_at_ymid(points), 0.5, 1.0]
    }
}

/// One-site saturation binding `B = Bmax * X / (Kd + X)` (rectangular hyperbola), optionally with
/// a Hill exponent (`Bmax * X^h / (Kd^h + X^h)`) or with linear nonspecific binding plus background
/// (`+ NS * X + Background`).
/// Reference: Langmuir (1918) isotherm / mass-action binding; Hill (1910) for the Hill variant;
/// see Hulme & Trevethick (2010) and Motulsky & Christopoulos (2004).
struct OneSiteBinding {
    meta: ModelMeta,
    with_hill: bool,
    with_ns: bool,
}

fn eval_hill_binding(c: f64, scale: f64, kd: f64, h: f64) -> Option<f64> {
    if !c.is_finite() || c < 0.0 || !scale.is_finite() || !kd.is_finite() || kd <= 0.0 || !h.is_finite() {
        return None;
    }
    if c == 0.0 {
        return (h > 0.0).then_some(0.0);
    }
    let base = c / kd;
    let ratio = if base.is_finite() && base > 0.0 {
        base.powf(h)
    } else {
        let log_arg = h * (c.ln() - kd.ln());
        if !log_arg.is_finite() {
            return None;
        }
        if log_arg > 709.0 {
            f64::INFINITY
        } else if log_arg < -745.0 {
            0.0
        } else {
            log_arg.exp()
        }
    };
    if !ratio.is_finite() {
        return (ratio.is_infinite() && h > 0.0).then_some(scale);
    }
    if ratio == 0.0 {
        return Some(0.0);
    }
    let y = scale * (ratio / (1.0 + ratio));
    y.is_finite().then_some(y)
}

impl ParametricModel for OneSiteBinding {
    impl_meta_methods!();
    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let c = x.first().copied()?;
        if c < 0.0 || !c.is_finite() {
            return None;
        }
        let (bmax, kd) = (params[0], params[1]);
        if !bmax.is_finite() || !kd.is_finite() || kd <= 0.0 {
            return None;
        }
        if self.with_hill {
            return eval_hill_binding(c, bmax, kd, params[2]);
        }
        let frac = if c == 0.0 {
            0.0
        } else {
            let r = c / kd;
            if r.is_infinite() {
                1.0
            } else if r == 0.0 {
                let log_r = c.ln() - kd.ln();
                if log_r < -745.0 {
                    0.0
                } else {
                    let rv = log_r.exp();
                    rv / (1.0 + rv)
                }
            } else {
                r / (1.0 + r)
            }
        };
        let spec = bmax * frac;
        let y = if self.with_ns {
            let (ns, bg) = (params[2], params[3]);
            if !ns.is_finite() || !bg.is_finite() {
                return None;
            }
            spec + ns * c + bg
        } else {
            spec
        };
        y.is_finite().then_some(y)
    }
    fn grad(&self, x: &[f64], params: &[f64], _dataset_index: usize, out: &mut [f64]) -> bool {
        if self.with_hill {
            return false;
        }
        let Some(&c) = x.first() else {
            return false;
        };
        let (bmax, kd) = (params[0], params[1]);
        let denom = kd + c;
        if c < 0.0 || denom <= 0.0 {
            return false;
        }
        out[0] = c / denom;
        out[1] = -(bmax * c) / (denom * denom);
        if self.with_ns {
            out[2] = c;
            out[3] = 1.0;
        }
        out.iter().all(|v| v.is_finite())
    }
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (ymin, ymax) = data_y_min_max(points);
        let kd = estimate_x_at_ymid(points).abs().max(1e-3);
        if self.with_hill {
            vec![ymax, kd, 1.0]
        } else if self.with_ns {
            vec![(ymax - ymin).max(1.0), kd, 0.0, ymin]
        } else {
            vec![ymax.max(1.0), kd]
        }
    }
    fn default_bounds(&self) -> Vec<(Option<f64>, Option<f64>)> {
        let mut b = vec![(Some(0.0), None), (Some(1e-30), None)];
        if self.with_hill {
            b.push((Some(0.05), Some(20.0)));
        } else if self.with_ns {
            b.push((Some(0.0), None));
            b.push((None, None));
        }
        b
    }
}

/// Two independent classes of sites: sum of two `Bmax * X / (Kd + X)` hyperbolas.
/// Reference: Langmuir (1918) isotherm summed over two site classes; Hulme & Trevethick (2010).
struct TwoSiteBinding;

impl ParametricModel for TwoSiteBinding {
    impl_static_meta!(
        "two_sites_specific_binding",
        "Two-site specific binding",
        EquationCategory::ReceptorBinding,
        ["BmaxHi", "KdHi", "BmaxLo", "KdLo"]
    );
    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let c = x.first().copied()?;
        let (b1, kd1, b2, kd2) = (params[0], params[1], params[2], params[3]);
        if c < 0.0 || kd1 <= 0.0 || kd2 <= 0.0 {
            return None;
        }
        Some(b1 * c / (kd1 + c) + b2 * c / (kd2 + c))
    }
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (_, ymax) = data_y_min_max(points);
        let kd = estimate_x_at_ymid(points).abs().max(1e-3);
        vec![0.5 * ymax, 0.2 * kd, 0.5 * ymax, 5.0 * kd]
    }
}

/// One-site competition (Hill slope 1) with `LogIC50 = LogKi + log10(1 + [hot] / Kd_hot)`.
/// Reference: Cheng & Prusoff (1973); log-X form per Motulsky & Christopoulos (2004).
struct CompetitionOneSiteKi;

impl ParametricModel for CompetitionOneSiteKi {
    impl_static_meta!(
        "one_site_fit_ki",
        "One-site competitive binding, Ki by Cheng–Prusoff",
        EquationCategory::ReceptorBinding,
        ["Top", "Bottom", "LogKi", "HotConc_nM", "HotKd_nM"]
    );
    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let x0 = x.first().copied()?;
        let (top, bottom, log_ki, hot_conc, hot_kd) = (params[0], params[1], params[2], params[3], params[4]);
        if hot_conc < 0.0 || hot_kd <= 0.0 {
            return None;
        }
        let log_ic50 = log_ki + (1.0 + hot_conc / hot_kd).log10();
        let (sig, _) = sigmoid_from_log_x(x0, log_ic50, -1.0)?;
        Some(bottom + (top - bottom) * sig)
    }
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (ymin, ymax) = data_y_min_max(points);
        vec![ymax, ymin, estimate_x_at_ymid(points), 1.0, 1.0]
    }
}

/// Two-site competition: `FractionHi`-weighted sum of two one-site curves, each IC50 converted to
/// Ki with the Cheng & Prusoff (1973) correction (assumes the radioligand has the same `Kd` at
/// both sites). Reference: Motulsky & Christopoulos (2004).
struct CompetitionTwoSiteKi;

impl ParametricModel for CompetitionTwoSiteKi {
    impl_static_meta!(
        "two_sites_fit_ki",
        "Two-site competitive binding (Ki)",
        EquationCategory::ReceptorBinding,
        ["Top", "Bottom", "FractionHi", "LogKi_1", "LogKi_2", "HotConc_nM", "HotKd_nM"]
    );
    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let x0 = x.first().copied()?;
        let (top, bottom, f_hi, log_ki1, log_ki2, hot_conc, hot_kd) = (params[0], params[1], params[2], params[3], params[4], params[5], params[6]);
        if hot_conc < 0.0 || hot_kd <= 0.0 {
            return None;
        }
        let shift = (1.0 + hot_conc / hot_kd).log10();
        let (s1, _) = sigmoid_from_log_x(x0, log_ki1 + shift, -1.0)?;
        let (s2, _) = sigmoid_from_log_x(x0, log_ki2 + shift, -1.0)?;
        Some(bottom + (top - bottom) * (f_hi * s1 + (1.0 - f_hi) * s2))
    }
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (ymin, ymax) = data_y_min_max(points);
        let m = estimate_x_at_ymid(points);
        vec![ymax, ymin, 0.5, m - 1.0, m + 1.0, 1.0, 1.0]
    }
}

/// Single-exponential dissociation `Y = (Y0 - NS) * exp(-Koff * t) + NS`.
/// Reference: mass-action binding kinetics, Motulsky & Christopoulos (2004).
struct DissociationOnePhase;

impl ParametricModel for DissociationOnePhase {
    impl_static_meta!("dissociation_one_phase", "One-phase exponential dissociation", EquationCategory::ReceptorBinding, ["Y0", "NS", "Koff"]);
    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let t = x.first().copied()?;
        let (y0, ns, koff) = (params[0], params[1], params[2]);
        if !t.is_finite() || !y0.is_finite() || !ns.is_finite() || !koff.is_finite() {
            return None;
        }
        let e = safe_exp(-koff * t)?;
        let y = (y0 - ns) * e + ns;
        y.is_finite().then_some(y)
    }
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (ymin, ymax) = data_y_min_max(points);
        let (xmin, xmax) = data_x_min_max(points);
        let span = (xmax - xmin).max(1.0);
        vec![ymax, ymin, 1.0 / span]
    }
}

/// Single-exponential association `Y = Ymax * (1 - exp(-Kobs * t))`.
/// Reference: mass-action binding kinetics, Motulsky & Christopoulos (2004).
struct AssociationOneConc;

impl ParametricModel for AssociationOneConc {
    impl_static_meta!(
        "association_kinetics_one_conc",
        "One phase association (one concentration of hot)",
        EquationCategory::ReceptorBinding,
        ["Ymax", "Kobs"]
    );
    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let t = x.first().copied()?;
        let (ymax, kobs) = (params[0], params[1]);
        if !t.is_finite() || !ymax.is_finite() || !kobs.is_finite() {
            return None;
        }
        let e = safe_exp(-kobs * t)?;
        let y = ymax * (1.0 - e);
        y.is_finite().then_some(y)
    }
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (_, ymax) = data_y_min_max(points);
        let (xmin, xmax) = data_x_min_max(points);
        vec![ymax, 2.0 / (xmax - xmin).max(1.0)]
    }
}

/// Association at radioligand concentration `L` with `Kobs = Kon * L + Koff` and plateau
/// `Bmax * L / (L + Koff / Kon)`.
/// Reference: mass-action binding kinetics, Motulsky & Christopoulos (2004); this is the
/// competitor-free special case of Motulsky & Mahan (1984).
struct AssociationTwoConc;

impl ParametricModel for AssociationTwoConc {
    impl_static_meta!(
        "association_kinetics_two_conc",
        "Association kinetics at multiple radioligand concentrations",
        EquationCategory::ReceptorBinding,
        ["Bmax", "Kon", "Koff", "LigandConc"]
    );
    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let t = x.first().copied()?;
        let (bmax, kon, koff, conc) = (params[0], params[1], params[2], params[3]);
        if !t.is_finite() || !bmax.is_finite() || !kon.is_finite() || !koff.is_finite() || !conc.is_finite() {
            return None;
        }
        let kobs = kon * conc + koff;
        if !kobs.is_finite() || kobs <= 0.0 {
            return None;
        }
        let e = safe_exp(-kobs * t)?;
        let eq = bmax * (kon * conc / kobs);
        let y = eq * (1.0 - e);
        y.is_finite().then_some(y)
    }
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (_, ymax) = data_y_min_max(points);
        vec![ymax, 1e6, 0.01, 1e-8]
    }
}

/// Association (as `association_kinetics_two_conc`) until `Time0`, then dissociation at `Koff`
/// from the level reached at `Time0`.
/// Reference: mass-action binding kinetics, Motulsky & Christopoulos (2004).
struct AssocThenDissoc;

impl ParametricModel for AssocThenDissoc {
    impl_static_meta!(
        "assoc_then_dissoc",
        "Association then dissociation",
        EquationCategory::ReceptorBinding,
        ["Bmax", "Kon", "Koff", "LigandConc", "Time0"]
    );
    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let t = x.first().copied()?;
        let (bmax, kon, koff, conc, t0) = (params[0], params[1], params[2], params[3], params[4]);
        if !t.is_finite() || !bmax.is_finite() || !kon.is_finite() || !koff.is_finite() || !conc.is_finite() || !t0.is_finite() {
            return None;
        }
        let kobs = kon * conc + koff;
        if !kobs.is_finite() || kobs <= 0.0 {
            return None;
        }
        let eq = bmax * (kon * conc / kobs);
        if !eq.is_finite() {
            return None;
        }
        let y = if t < t0 {
            let e = safe_exp(-kobs * t)?;
            eq * (1.0 - e)
        } else {
            let e_t0 = safe_exp(-kobs * t0)?;
            let bound_at_t0 = eq * (1.0 - e_t0);
            let e_dissoc = safe_exp(-koff * (t - t0))?;
            bound_at_t0 * e_dissoc
        };
        y.is_finite().then_some(y)
    }
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (_, ymax) = data_y_min_max(points);
        let (xmin, xmax) = data_x_min_max(points);
        vec![ymax, 1e6, 0.05, 1e-8, 0.5 * (xmin + xmax)]
    }
}

/// Michaelis–Menten `v = Vmax * S / (Km + S)`; the `kcat` variant uses `Vmax = kcat * Et`.
/// Reference: Michaelis & Menten (1913); Segel (1975).
struct MichaelisMenten {
    meta: ModelMeta,
    kcat_mode: bool,
}

impl ParametricModel for MichaelisMenten {
    impl_meta_methods!();
    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let s = x.first().copied()?;
        if s < 0.0 || !s.is_finite() {
            return None;
        }
        if self.kcat_mode {
            let (kcat, km, et) = (params[0], params[1], params[2]);
            if km <= 0.0 || km + s == 0.0 {
                return None;
            }
            Some(et * kcat * s / (km + s))
        } else {
            let (vmax, km) = (params[0], params[1]);
            if km <= 0.0 || km + s == 0.0 {
                return None;
            }
            Some(vmax * s / (km + s))
        }
    }
    fn grad(&self, x: &[f64], params: &[f64], _dataset_index: usize, out: &mut [f64]) -> bool {
        let Some(&s) = x.first() else {
            return false;
        };
        if s < 0.0 {
            return false;
        }
        if self.kcat_mode {
            let (kcat, km, et) = (params[0], params[1], params[2]);
            let d = km + s;
            if d <= 0.0 {
                return false;
            }
            out[0] = et * s / d;
            out[1] = -(et * kcat * s) / (d * d);
            out[2] = kcat * s / d;
        } else {
            let (vmax, km) = (params[0], params[1]);
            let d = km + s;
            if d <= 0.0 {
                return false;
            }
            out[0] = s / d;
            out[1] = -(vmax * s) / (d * d);
        }
        out.iter().all(|v| v.is_finite())
    }
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (_, ymax) = data_y_min_max(points);
        let v_init = if ymax > 0.0 && ymax.is_finite() { ymax } else { 1.0 };
        let km = estimate_x_at_ymid(points).abs().max(1e-3);
        if self.kcat_mode { vec![v_init, km, 1.0] } else { vec![v_init, km] }
    }
    fn default_bounds(&self) -> Vec<(Option<f64>, Option<f64>)> {
        if self.kcat_mode { vec![(Some(0.0), None), (Some(1e-30), None), (Some(0.0), None)] } else { vec![(Some(0.0), None), (Some(1e-30), None)] }
    }
}

/// Sigmoidal (Hill-type) kinetics `v = Vmax * S^h / (Khalf^h + S^h)`.
/// Reference: Hill (1910); Segel (1975).
struct AllostericSigmoidal;

impl ParametricModel for AllostericSigmoidal {
    impl_static_meta!("allosteric_sigmoidal_kinetics", "Allosteric sigmoidal", EquationCategory::EnzymeKinetics, ["Vmax", "Khalf", "h"]);
    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let s = x.first().copied()?;
        let (vmax, khalf, h) = (params[0], params[1], params[2]);
        eval_hill_binding(s, vmax, khalf, h)
    }
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (_, ymax) = data_y_min_max(points);
        vec![ymax.max(1.0), estimate_x_at_ymid(points).abs().max(1e-3), 1.5]
    }
}

/// Uncompetitive substrate inhibition `v = Vmax * S / (Km + S * (1 + S / Ki))`.
/// Reference: Haldane (1930); Segel (1975).
struct SubstrateInhibition;

impl ParametricModel for SubstrateInhibition {
    impl_static_meta!("substrate_inhibition", "Substrate inhibition", EquationCategory::EnzymeKinetics, ["Vmax", "Km", "Ki"]);
    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let s = x.first().copied()?;
        let (vmax, km, ki) = (params[0], params[1], params[2]);
        if s < 0.0 || km <= 0.0 || ki <= 0.0 {
            return None;
        }
        Some(vmax * s / (km + s * (1.0 + s / ki)))
    }
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (_, ymax) = data_y_min_max(points);
        let (_, xmax) = data_x_min_max(points);
        vec![ymax.max(1.0) * 1.5, estimate_x_at_ymid(points).abs().max(1e-3), xmax.abs().max(1.0)]
    }
}

#[derive(Clone, Copy)]
enum EnzymeInhibMode {
    Competitive,
    Noncompetitive,
    Uncompetitive,
    Mixed,
}

/// Rapid-equilibrium reversible inhibition at inhibitor concentration `I`:
/// competitive (`Km_app = Km (1 + I/Ki)`), pure noncompetitive (`Vmax_app = Vmax / (1 + I/Ki)`),
/// uncompetitive (`Vmax` and `Km` both divided by `1 + I/(alpha Ki)`), and mixed
/// (`Vmax_app = Vmax / (1 + I/(alpha Ki))`, `Km_app = Km (1 + I/Ki) / (1 + I/(alpha Ki))`).
/// Reference: Segel (1975).
struct EnzymeInhibition {
    meta: ModelMeta,
    mode: EnzymeInhibMode,
}

impl ParametricModel for EnzymeInhibition {
    impl_meta_methods!();
    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let s = x.first().copied()?;
        if s < 0.0 {
            return None;
        }
        match self.mode {
            EnzymeInhibMode::Competitive => {
                let (vmax, km, ki, i_conc) = (params[0], params[1], params[2], params[3]);
                if km <= 0.0 || ki <= 0.0 {
                    return None;
                }
                let km_app = km * (1.0 + i_conc / ki);
                Some(vmax * s / (km_app + s))
            }
            EnzymeInhibMode::Noncompetitive => {
                let (vmax, km, ki, i_conc) = (params[0], params[1], params[2], params[3]);
                if km <= 0.0 || ki <= 0.0 {
                    return None;
                }
                let vmax_app = vmax / (1.0 + i_conc / ki);
                Some(vmax_app * s / (km + s))
            }
            EnzymeInhibMode::Uncompetitive => {
                let (vmax, km, aki, i_conc) = (params[0], params[1], params[2], params[3]);
                if km <= 0.0 || aki <= 0.0 {
                    return None;
                }
                let factor = 1.0 + i_conc / aki;
                Some((vmax / factor) * s / (km / factor + s))
            }
            EnzymeInhibMode::Mixed => {
                let (vmax, km, ki, alpha, i_conc) = (params[0], params[1], params[2], params[3], params[4]);
                if km <= 0.0 || ki <= 0.0 || alpha <= 0.0 {
                    return None;
                }
                let vmax_app = vmax / (1.0 + i_conc / (alpha * ki));
                let km_app = km * (1.0 + i_conc / ki) / (1.0 + i_conc / (alpha * ki));
                Some(vmax_app * s / (km_app + s))
            }
        }
    }
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (_, ymax) = data_y_min_max(points);
        let km = estimate_x_at_ymid(points).abs().max(1e-3);
        match self.mode {
            EnzymeInhibMode::Mixed => vec![ymax.max(1.0), km, 1.0, 1.0, 0.0],
            _ => vec![ymax.max(1.0), km, 1.0, 0.0],
        }
    }
}

/// Morrison tight-binding inhibition (X = total inhibitor concentration):
/// `v = Vo * (1 - ((Et + I + Ki_app) - sqrt((Et + I + Ki_app)^2 - 4 Et I)) / (2 Et))`, with
/// `Ki_app = Ki * (1 + S / Km)` (competitive inhibitor).
/// Reference: Morrison (1969).
struct MorrisonKi;

impl ParametricModel for MorrisonKi {
    impl_static_meta!("morrison_tight_binding_ki", "Morrison tight-binding Ki", EquationCategory::EnzymeKinetics, ["Vo", "Et", "Ki", "S", "Km"]);
    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let i_conc = x.first().copied()?;
        let (vo, et, ki, s_conc, km) = (params[0], params[1], params[2], params[3], params[4]);
        if !i_conc.is_finite()
            || i_conc < 0.0
            || !vo.is_finite()
            || !et.is_finite()
            || et <= 0.0
            || !ki.is_finite()
            || ki <= 0.0
            || !s_conc.is_finite()
            || s_conc < 0.0
            || !km.is_finite()
            || km <= 0.0
        {
            return None;
        }
        let ki_app = ki * (1.0 + s_conc / km);
        if !ki_app.is_finite() || ki_app <= 0.0 {
            return None;
        }
        if i_conc == 0.0 {
            return Some(vo);
        }
        let max_term = et.max(i_conc).max(ki_app);
        let (et_s, i_s, k_s) =
            if max_term > 1e150 || max_term < 1e-150 { (et / max_term, i_conc / max_term, ki_app / max_term) } else { (et, i_conc, ki_app) };
        let disc = (et_s - i_s + k_s).hypot(2.0 * i_s.sqrt() * k_s.sqrt());
        if !disc.is_finite() {
            return None;
        }
        let b = i_s + k_s - et_s;
        let frac = if b >= 0.0 {
            let denom = b + disc;
            if denom <= 0.0 || !denom.is_finite() {
                return None;
            }
            (2.0 * k_s) / denom
        } else {
            ((et_s - i_s - k_s) + disc) / (2.0 * et_s)
        };
        let y = vo * frac;
        y.is_finite().then_some(y)
    }
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (_, ymax) = data_y_min_max(points);
        vec![ymax.max(1.0), 1.0, 0.1, 1.0, 1.0]
    }
}

fn split_total_and_ns_datasets(points: &[DataPoint]) -> (Vec<DataPoint>, Vec<DataPoint>) {
    let ds0 = points.iter().filter(|p| p.data_set_index == 0).cloned().collect();
    let ds1 = points.iter().filter(|p| p.data_set_index >= 1).cloned().collect();
    (ds0, ds1)
}

fn estimate_global_ns_and_bg(primary: &[DataPoint], ds1: &[DataPoint]) -> (f64, f64) {
    let (ymin, ymax) = data_y_min_max(primary);
    let (_, xmax) = data_x_min_max(primary);
    let finite_ds1: Vec<(f64, f64)> = ds1.iter().filter(|p| p.x0().is_finite() && p.y.is_finite()).map(|p| (p.x0(), p.y)).collect();
    if finite_ds1.len() >= 2 {
        let n = finite_ds1.len() as f64;
        let mx = finite_ds1.iter().map(|&(x, _)| x).sum::<f64>() / n;
        let my = finite_ds1.iter().map(|&(_, y)| y).sum::<f64>() / n;
        let mut sxx = 0.0_f64;
        let mut sxy = 0.0_f64;
        for &(x, y) in &finite_ds1 {
            let dx = x - mx;
            sxx += dx * dx;
            sxy += dx * (y - my);
        }
        let slope = if sxx > 1e-30 && sxx.is_finite() && sxy.is_finite() { (sxy / sxx).max(0.0) } else { 0.0 };
        let bg = my - slope * mx;
        if slope.is_finite() && bg.is_finite() {
            return (slope, bg);
        }
    }
    (((ymax - ymin) / xmax.max(1e-12)) * 0.05, ymin)
}

/// Global one-site total and nonspecific binding (`dataset 0` = Total, `dataset >= 1` = Nonspecific).
/// Reference: Langmuir (1918) specific term plus linear nonspecific binding; shared-parameter
/// global fit per Motulsky & Christopoulos (2004).
struct OneSiteTotalAndNsGlobal;

impl ParametricModel for OneSiteTotalAndNsGlobal {
    impl_static_meta!(
        "one_site_total_and_ns_global",
        "One-site total and nonspecific binding (global)",
        EquationCategory::ReceptorBinding,
        ["Bmax", "Kd", "NS", "Background"]
    );

    fn eval(&self, x: &[f64], params: &[f64], dataset_index: usize) -> Option<f64> {
        let x0 = x.first().copied()?;
        if !x0.is_finite() || x0 < 0.0 || params.len() < 4 || !params[..4].iter().all(|v| v.is_finite()) {
            return None;
        }
        let [bmax, kd, ns, bg] = [params[0], params[1], params[2], params[3]];
        if dataset_index == 0 {
            if kd <= 0.0 {
                return None;
            }
            let y = bmax * x0 / (kd + x0) + ns * x0 + bg;
            y.is_finite().then_some(y)
        } else {
            let y = ns * x0 + bg;
            y.is_finite().then_some(y)
        }
    }

    fn grad(&self, x: &[f64], params: &[f64], dataset_index: usize, out: &mut [f64]) -> bool {
        let Some(&x0) = x.first() else {
            return false;
        };
        if !x0.is_finite() || x0 < 0.0 || params.len() < 4 || out.len() < 4 || !params[..4].iter().all(|v| v.is_finite()) {
            return false;
        }
        if dataset_index == 0 {
            let [bmax, kd] = [params[0], params[1]];
            if kd <= 0.0 {
                return false;
            }
            let denom = kd + x0;
            out[0] = x0 / denom;
            out[1] = -bmax * x0 / (denom * denom);
            out[2] = x0;
            out[3] = 1.0;
        } else {
            out[0] = 0.0;
            out[1] = 0.0;
            out[2] = x0;
            out[3] = 1.0;
        }
        out[..4].iter().all(|v| v.is_finite())
    }

    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (ds0, ds1) = split_total_and_ns_datasets(points);
        let primary = if ds0.is_empty() { points } else { &ds0 };
        let (ymin, ymax) = data_y_min_max(primary);
        let (_, xmax) = data_x_min_max(primary);
        let kd = estimate_x_at_ymid(primary).max(1e-6);
        let (ns, bg) = estimate_global_ns_and_bg(primary, &ds1);
        let bmax = (ymax - bg - ns * xmax).max(ymax - ymin).max(1.0);
        vec![bmax, kd, ns, bg]
    }

    fn default_bounds(&self) -> Vec<(Option<f64>, Option<f64>)> {
        vec![(Some(0.0), None), (Some(1e-12), None), (Some(0.0), None), (None, None)]
    }
}

/// Global two-site total and nonspecific binding (`dataset 0` = Total, `dataset >= 1` = Nonspecific).
/// Reference: two-site Langmuir (1918) specific term plus linear nonspecific binding; global fit
/// per Motulsky & Christopoulos (2004).
struct TwoSiteTotalAndNsGlobal;

impl ParametricModel for TwoSiteTotalAndNsGlobal {
    impl_static_meta!(
        "two_site_total_and_ns_global",
        "Two-site total and nonspecific binding (global)",
        EquationCategory::ReceptorBinding,
        ["BmaxHi", "KdHi", "BmaxLo", "KdLo", "NS", "Background"]
    );

    fn eval(&self, x: &[f64], params: &[f64], dataset_index: usize) -> Option<f64> {
        let x0 = x.first().copied()?;
        if !x0.is_finite() || x0 < 0.0 || params.len() < 6 || !params[..6].iter().all(|v| v.is_finite()) {
            return None;
        }
        let [bmax_hi, kd_hi, bmax_lo, kd_lo, ns, bg] = [params[0], params[1], params[2], params[3], params[4], params[5]];
        if dataset_index == 0 {
            if kd_hi <= 0.0 || kd_lo <= 0.0 {
                return None;
            }
            let y = bmax_hi * x0 / (kd_hi + x0) + bmax_lo * x0 / (kd_lo + x0) + ns * x0 + bg;
            y.is_finite().then_some(y)
        } else {
            let y = ns * x0 + bg;
            y.is_finite().then_some(y)
        }
    }

    fn grad(&self, x: &[f64], params: &[f64], dataset_index: usize, out: &mut [f64]) -> bool {
        let Some(&x0) = x.first() else {
            return false;
        };
        if !x0.is_finite() || x0 < 0.0 || params.len() < 6 || out.len() < 6 || !params[..6].iter().all(|v| v.is_finite()) {
            return false;
        }
        if dataset_index == 0 {
            let [bmax_hi, kd_hi, bmax_lo, kd_lo] = [params[0], params[1], params[2], params[3]];
            if kd_hi <= 0.0 || kd_lo <= 0.0 {
                return false;
            }
            let denom_hi = kd_hi + x0;
            let denom_lo = kd_lo + x0;
            out[0] = x0 / denom_hi;
            out[1] = -bmax_hi * x0 / (denom_hi * denom_hi);
            out[2] = x0 / denom_lo;
            out[3] = -bmax_lo * x0 / (denom_lo * denom_lo);
            out[4] = x0;
            out[5] = 1.0;
        } else {
            out[0] = 0.0;
            out[1] = 0.0;
            out[2] = 0.0;
            out[3] = 0.0;
            out[4] = x0;
            out[5] = 1.0;
        }
        out[..6].iter().all(|v| v.is_finite())
    }

    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (ds0, ds1) = split_total_and_ns_datasets(points);
        let primary = if ds0.is_empty() { points } else { &ds0 };
        let (ymin, ymax) = data_y_min_max(primary);
        let (_, xmax) = data_x_min_max(primary);
        let (ns, bg) = estimate_global_ns_and_bg(primary, &ds1);
        let spec_pts: Vec<DataPoint> = primary
            .iter()
            .map(|p| {
                let mut q = p.clone();
                q.y = p.y - (ns * p.x0() + bg);
                q
            })
            .collect();
        let (_, spec_ymax) = data_y_min_max(&spec_pts);
        let bmax_tot = spec_ymax.max(ymax - bg - ns * xmax).max(ymax - ymin).max(1.0);
        let raw_kd = estimate_x_at_ymid(&spec_pts);
        let kd_mid = if raw_kd.is_finite() && raw_kd > 0.0 { raw_kd } else { 0.2 * xmax.max(1e-9) };
        let kd_hi = (0.2 * kd_mid).max(1e-12);
        let kd_lo = (5.0 * kd_mid).max(1e-12);
        vec![0.5 * bmax_tot, kd_hi, 0.5 * bmax_tot, kd_lo, ns, bg]
    }

    fn default_bounds(&self) -> Vec<(Option<f64>, Option<f64>)> {
        vec![(Some(0.0), None), (Some(1e-15), None), (Some(0.0), None), (Some(1e-15), None), (Some(0.0), None), (None, None)]
    }
}

/// Specific saturation binding with ligand depletion (Swillens 1995 / Morrison quadratic root).
/// Free ligand solves `Total = Free * (1 + NS) + Rt * Free / (Kd + Free)`.
/// Reference: Swillens (1995); quadratic-root treatment as in Morrison (1969).
struct OneSiteSpecificLigandDepletion;

impl OneSiteSpecificLigandDepletion {
    fn solve_free_ligand(hot_tot: f64, rt: f64, kd: f64, ns: f64) -> Option<(f64, f64)> {
        let a = 1.0 + ns;
        if a <= 0.0 || kd <= 0.0 || rt < 0.0 || hot_tot < 0.0 {
            return None;
        }
        if hot_tot == 0.0 {
            return Some((0.0, a));
        }
        let b = kd * a + rt - hot_tot;
        let disc = (b * b + 4.0 * a * kd * hot_tot).sqrt();
        if !disc.is_finite() {
            return None;
        }
        let l_free = if b >= 0.0 { (2.0 * kd * hot_tot) / (b + disc) } else { (-b + disc) / (2.0 * a) };
        l_free.is_finite().then_some((l_free, a))
    }
}

impl ParametricModel for OneSiteSpecificLigandDepletion {
    impl_static_meta!(
        "one_site_specific_ligand_depletion",
        "One-site specific binding with ligand depletion",
        EquationCategory::ReceptorBinding,
        ["Rt", "Kd", "NS"]
    );

    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let hot_tot = x.first().copied()?;
        if !hot_tot.is_finite() || hot_tot < 0.0 || params.len() < 3 || !params[..3].iter().all(|v| v.is_finite()) {
            return None;
        }
        let [rt, kd, ns] = [params[0], params[1], params[2]];
        let (l_free, _) = Self::solve_free_ligand(hot_tot, rt, kd, ns)?;
        let y = rt * l_free / (kd + l_free);
        y.is_finite().then_some(y)
    }

    fn grad(&self, x: &[f64], params: &[f64], _dataset_index: usize, out: &mut [f64]) -> bool {
        let Some(&hot_tot) = x.first() else {
            return false;
        };
        if !hot_tot.is_finite() || hot_tot < 0.0 || params.len() < 3 || out.len() < 3 || !params[..3].iter().all(|v| v.is_finite()) {
            return false;
        }
        let [rt, kd, ns] = [params[0], params[1], params[2]];
        let Some((l_free, a)) = Self::solve_free_ligand(hot_tot, rt, kd, ns) else {
            return false;
        };
        let denom = kd + l_free;
        let p = l_free / denom;
        let q = kd / denom;
        let df_dl = a + rt * q / denom;
        if df_dl <= 0.0 || !df_dl.is_finite() {
            return false;
        }
        let y = rt * p;
        out[0] = (a * p) / df_dl;
        out[1] = -(a * rt * p) / (denom * df_dl);
        out[2] = -(y * q) / df_dl;
        out[..3].iter().all(|v| v.is_finite())
    }

    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (_, ymax) = data_y_min_max(points);
        let kd = estimate_x_at_ymid(points).max(1e-6);
        vec![ymax.max(1.0), kd, 0.0]
    }

    fn default_bounds(&self) -> Vec<(Option<f64>, Option<f64>)> {
        vec![(Some(0.0), None), (Some(1e-12), None), (Some(0.0), None)]
    }
}

struct CompetitionDepletionState {
    r: f64,
    kd_nm: f64,
    ns: f64,
    kd: f64,
    hot_nm: f64,
    ki: f64,
    i_tot: f64,
}

#[inline]
fn eval_competition_mass_balance(r: f64, rt: f64, hot_nm: f64, kd: f64, i_tot: f64, ki: f64) -> (f64, f64) {
    let d_d = kd + r;
    let d_i = ki + r;
    let p_d = r / d_d;
    let q_d = kd / d_d;
    let p_i = r / d_i;
    let q_i = ki / d_i;
    let g = (r - rt) + p_d * hot_nm + p_i * i_tot;
    let gp = 1.0 + (hot_nm / d_d) * q_d + (i_tot / d_i) * q_i;
    (g, gp)
}

fn tighten_competition_bracket(rt: f64, hot_nm: f64, kd: f64, i_tot: f64, ki: f64) -> (f64, f64) {
    let mut lo = 0.0_f64;
    let mut hi = rt;
    let inv_lo = 1.0 + hot_nm / kd + i_tot / ki;
    if inv_lo.is_finite() && inv_lo > 0.0 {
        let cand_lo = (rt / inv_lo).min(rt);
        if cand_lo > 0.0 {
            let (g_lo, _) = eval_competition_mass_balance(cand_lo, rt, hot_nm, kd, i_tot, ki);
            if g_lo.is_finite() {
                if g_lo <= 0.0 {
                    lo = cand_lo;
                } else {
                    hi = cand_lo;
                }
            }
        }
    }
    let inv_hi = 1.0 + hot_nm / (kd + rt) + i_tot / (ki + rt);
    if inv_hi.is_finite() && inv_hi > 0.0 {
        let cand_hi = (rt / inv_hi).clamp(lo, hi);
        if cand_hi > lo && cand_hi < hi {
            let (g_hi, _) = eval_competition_mass_balance(cand_hi, rt, hot_nm, kd, i_tot, ki);
            if g_hi.is_finite() {
                if g_hi >= 0.0 {
                    hi = cand_hi;
                } else {
                    lo = cand_hi;
                }
            }
        }
    }
    (lo, hi)
}

/// One-site competitive binding with depletion of both radioligand and unlabeled competitor (`X = log10[Competitor M]`).
/// Solves the exact mass balance `Rt = R + Hot * R / (Kd * (1 + NS) + R) + I * R / (Ki + R)` numerically
/// for free receptor `R`; `Y` is the total bound radioligand (specific + nonspecific `NS * FreeHot`),
/// `Hot * (R + NS * Kd) / (Kd * (1 + NS) + R)`, where `FreeHot = Hot * Kd / (Kd * (1 + NS) + R)`.
/// Reference: exact two-ligand competition with depletion and nonspecific binding, Swillens (1995);
/// see also Wang (1995).
struct OneSiteCompetitionLigandDepletion;

impl OneSiteCompetitionLigandDepletion {
    fn solve_free_receptor(x0: f64, params: &[f64]) -> Option<CompetitionDepletionState> {
        if !x0.is_finite() || params.len() < 5 || !params[..5].iter().all(|v| v.is_finite()) {
            return None;
        }
        let [rt, hot_nm, kd_nm, log_ki, ns] = [params[0], params[1], params[2], params[3], params[4]];
        if rt < 0.0 || hot_nm < 0.0 || kd_nm <= 0.0 || ns < 0.0 {
            return None;
        }
        let i_tot = safe_pow10(x0 + 9.0)?;
        let ki = safe_pow10(log_ki + 9.0)?;
        let kd = kd_nm * (1.0 + ns);
        if !kd.is_finite() || kd <= 0.0 || ki <= 0.0 {
            return None;
        }
        if rt == 0.0 {
            return Some(CompetitionDepletionState { r: 0.0, kd_nm, ns, kd, hot_nm, ki, i_tot });
        }
        let (mut lo, mut hi) = tighten_competition_bracket(rt, hot_nm, kd, i_tot, ki);
        let mut r = 0.5 * (lo + hi);
        let mut prev_step = (hi - lo).max(0.0);
        for _ in 0..64 {
            if (hi - lo) <= 1e-14 * r.max(1e-300) {
                break;
            }
            let (g, gp) = eval_competition_mass_balance(r, rt, hot_nm, kd, i_tot, ki);
            if !g.is_finite() || !gp.is_finite() || gp < 1.0 {
                let mid = 0.5 * (lo + hi);
                if mid == lo || mid == hi || mid == r {
                    break;
                }
                prev_step = (hi - lo).max(0.0);
                r = mid;
                continue;
            }
            if g == 0.0 {
                break;
            }
            if g < 0.0 {
                lo = r;
            } else {
                hi = r;
            }
            let delta = g / gp;
            let r_new = r - delta;
            let step_abs = delta.abs();
            if r_new.is_finite() && r_new > lo && r_new < hi && step_abs <= 0.5 * prev_step {
                prev_step = step_abs;
                r = r_new;
                if step_abs <= 1e-14 * r.max(1e-300) {
                    break;
                }
            } else {
                let mid = 0.5 * (lo + hi);
                prev_step = (hi - lo).max(0.0);
                if mid == lo || mid == hi || mid == r {
                    break;
                }
                r = mid;
            }
        }
        r.is_finite().then_some(CompetitionDepletionState { r, kd_nm, ns, kd, hot_nm, ki, i_tot })
    }
}

impl ParametricModel for OneSiteCompetitionLigandDepletion {
    impl_static_meta!(
        "one_site_competition_ligand_depletion",
        "One-site competitive binding with ligand depletion",
        EquationCategory::ReceptorBinding,
        ["Rt", "HotNM", "KdNM", "LogKi", "NS"]
    );

    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let x0 = x.first().copied()?;
        let st = Self::solve_free_receptor(x0, params)?;
        let y = ((st.r + st.ns * st.kd_nm) / (st.kd + st.r)) * st.hot_nm;
        y.is_finite().then_some(y)
    }

    fn grad(&self, x: &[f64], params: &[f64], _dataset_index: usize, out: &mut [f64]) -> bool {
        let Some(&x0) = x.first() else {
            return false;
        };
        if out.len() < 5 {
            return false;
        }
        let Some(st) = Self::solve_free_receptor(x0, params) else {
            return false;
        };
        let d_d = st.kd + st.r;
        let d_i = st.ki + st.r;
        let p_d = st.r / d_d;
        let q_d = st.kd / d_d;
        let q_i = st.ki / d_i;
        let one_plus_ns = 1.0 + st.ns;
        let alpha = (st.hot_nm / d_d) * q_d;
        let alpha_sp = (st.hot_nm / d_d) * (st.kd_nm / d_d);
        let beta = (st.i_tot / d_i) * q_i;
        let rest_g = 1.0 + beta;
        let g_prime = alpha + rest_g;
        if !g_prime.is_finite() || g_prime < 1.0 {
            return false;
        }
        // Y = (B*_sp + NS * HotNM) / (1 + NS) where B*_sp = HotNM * p_d, so all partials remain cancellation-free.
        out[0] = alpha_sp / g_prime;
        out[1] = (st.ns + p_d * (rest_g / g_prime)) / one_plus_ns;
        out[2] = -((st.hot_nm * p_d) / d_d) * (rest_g / g_prime);
        out[3] = alpha_sp * (core::f64::consts::LN_10 * st.r * beta) / g_prime;
        out[4] = ((st.hot_nm * st.kd_nm) / (d_d * one_plus_ns)) * ((alpha + q_d * rest_g) / g_prime);
        out[..5].iter().all(|v| v.is_finite())
    }

    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (_, ymax) = data_y_min_max(points);
        let rt_guess = if ymax.is_finite() && ymax > 0.0 { 2.0 * ymax } else { 1.0 };
        let log_ki = if points.is_empty() { -8.0 } else { estimate_x_at_ymid(points) };
        vec![rt_guess, 1.0, 1.0, log_ki, 0.0]
    }

    fn default_bounds(&self) -> Vec<(Option<f64>, Option<f64>)> {
        vec![(Some(0.0), None), (Some(0.0), None), (Some(1e-15), None), (None, None), (Some(0.0), None)]
    }
}

/// Allosteric ternary-complex saturation binding with modulator concentration `ModConc`:
/// `B = Bmax * X / (X + Kd * (1 + ModConc/Kb) / (1 + alpha * ModConc/Kb))`.
/// Reference: Ehlert (1988).
struct AllostericModulatorSaturation;

impl ParametricModel for AllostericModulatorSaturation {
    impl_static_meta!(
        "allosteric_modulator_saturation",
        "Allosteric modulator saturation binding",
        EquationCategory::ReceptorBinding,
        ["Bmax", "Kd", "alpha", "Kb", "ModConc"]
    );

    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let x0 = x.first().copied()?;
        if !x0.is_finite() || x0 < 0.0 || params.len() < 5 || !params[..5].iter().all(|v| v.is_finite()) {
            return None;
        }
        let [bmax, kd, alpha, kb, mod_conc] = [params[0], params[1], params[2], params[3], params[4]];
        if kd <= 0.0 || kb <= 0.0 || alpha < 0.0 || mod_conc < 0.0 {
            return None;
        }
        let u = mod_conc / kb;
        let num_factor = 1.0 + alpha * u;
        let mod_factor = 1.0 + u;
        let denom = kd * mod_factor + x0 * num_factor;
        if denom <= 0.0 {
            return None;
        }
        let y = bmax * x0 * num_factor / denom;
        y.is_finite().then_some(y)
    }

    fn grad(&self, x: &[f64], params: &[f64], _dataset_index: usize, out: &mut [f64]) -> bool {
        let Some(&x0) = x.first() else {
            return false;
        };
        if !x0.is_finite() || x0 < 0.0 || params.len() < 5 || out.len() < 5 || !params[..5].iter().all(|v| v.is_finite()) {
            return false;
        }
        let [bmax, kd, alpha, kb, mod_conc] = [params[0], params[1], params[2], params[3], params[4]];
        if kd <= 0.0 || kb <= 0.0 || alpha < 0.0 || mod_conc < 0.0 {
            return false;
        }
        let u = mod_conc / kb;
        let n = 1.0 + alpha * u;
        let m = 1.0 + u;
        let denom = kd * m + x0 * n;
        if denom <= 0.0 {
            return false;
        }
        let d2 = denom * denom;
        out[0] = (x0 * n) / denom;
        out[1] = -(bmax * x0 * n * m) / d2;
        out[2] = (bmax * x0 * kd * m * u) / d2;
        out[3] = -(bmax * x0 * kd * (alpha - 1.0) * u) / (kb * d2);
        out[4] = (bmax * x0 * kd * (alpha - 1.0)) / (kb * d2);
        out[..5].iter().all(|v| v.is_finite())
    }

    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (_, ymax) = data_y_min_max(points);
        let kd = estimate_x_at_ymid(points).max(1e-6);
        vec![ymax.max(1.0), kd, 1.0, 1e-6, 0.0]
    }

    fn default_bounds(&self) -> Vec<(Option<f64>, Option<f64>)> {
        vec![(Some(0.0), None), (Some(1e-12), None), (Some(0.0), None), (Some(1e-12), None), (Some(0.0), None)]
    }
}

/// Homologous one-site competitive binding (`Kd_cold == Kd_hot`, `X = log10[Cold]`):
/// `B = Bmax * Hot / (Hot + Cold + Kd) + NS`.
/// Reference: DeBlasi, O'Reilly & Motulsky (1989).
struct HomologousOneSiteCompetition;

impl ParametricModel for HomologousOneSiteCompetition {
    impl_static_meta!(
        "homologous_one_site_competition",
        "Homologous one-site competitive binding",
        EquationCategory::ReceptorBinding,
        ["Bmax", "Kd", "NS", "HotConc"]
    );

    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let x0 = x.first().copied()?;
        if !x0.is_finite() || params.len() < 4 || !params[..4].iter().all(|v| v.is_finite()) {
            return None;
        }
        let [bmax, kd, ns, hot_conc] = [params[0], params[1], params[2], params[3]];
        if kd <= 0.0 || hot_conc <= 0.0 {
            return None;
        }
        let cold = safe_pow10(x0)?;
        let denom = hot_conc + cold + kd;
        let y = bmax * hot_conc / denom + ns;
        y.is_finite().then_some(y)
    }

    fn grad(&self, x: &[f64], params: &[f64], _dataset_index: usize, out: &mut [f64]) -> bool {
        let Some(&x0) = x.first() else {
            return false;
        };
        if !x0.is_finite() || params.len() < 4 || out.len() < 4 || !params[..4].iter().all(|v| v.is_finite()) {
            return false;
        }
        let [bmax, kd, _, hot_conc] = [params[0], params[1], params[2], params[3]];
        if kd <= 0.0 || hot_conc <= 0.0 {
            return false;
        }
        let Some(cold) = safe_pow10(x0) else {
            return false;
        };
        let denom = hot_conc + cold + kd;
        let d2 = denom * denom;
        out[0] = hot_conc / denom;
        out[1] = -(bmax * hot_conc) / d2;
        out[2] = 1.0;
        out[3] = (bmax * (cold + kd)) / d2;
        out[..4].iter().all(|v| v.is_finite())
    }

    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (ymin, ymax) = data_y_min_max(points);
        let x_mid = estimate_x_at_ymid(points);
        let ic50 = safe_pow10(x_mid).unwrap_or(1e-9).max(1e-18);
        let hot = 0.5 * ic50;
        let kd = 0.5 * ic50;
        let bmax = 2.0 * (ymax - ymin).max(1.0);
        vec![bmax, kd, ymin, hot]
    }

    fn default_bounds(&self) -> Vec<(Option<f64>, Option<f64>)> {
        vec![(Some(0.0), None), (Some(1e-18), None), (None, None), (Some(1e-18), None)]
    }
}

/// Motulsky-Mahan (1984) closed-form competitive binding kinetics vs time `t = X`.
/// Reference: Motulsky & Mahan (1984), eq. for radioligand binding in the presence of an unlabeled
/// competitor (`kon1`/`koff1` radioligand, `kon2`/`koff2` competitor).
struct MotulskyMahanKinetics;

impl ParametricModel for MotulskyMahanKinetics {
    impl_static_meta!(
        "motulsky_mahan_kinetics",
        "Motulsky-Mahan competitive binding kinetics",
        EquationCategory::ReceptorBinding,
        ["Bmax", "kon1", "koff1", "kon2", "koff2", "L", "I"]
    );

    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let t = x.first().copied()?;
        if !t.is_finite() || t < 0.0 || params.len() < 7 || !params[..7].iter().all(|v| v.is_finite()) {
            return None;
        }
        let [bmax, kon1, koff1, kon2, koff2, l, i] = [params[0], params[1], params[2], params[3], params[4], params[5], params[6]];
        if bmax < 0.0 || kon1 < 0.0 || koff1 < 0.0 || kon2 < 0.0 || koff2 <= 0.0 || l < 0.0 || i < 0.0 {
            return None;
        }
        if t == 0.0 || kon1 == 0.0 || l == 0.0 {
            return Some(0.0);
        }
        let ka = kon1 * l + koff1;
        let kb = kon2 * i + koff2;
        let s = (ka - kb).hypot(2.0 * (kon1 * kon2 * l * i).sqrt());
        let kf = 0.5 * (ka + kb + s);
        let kf_ks = kon1 * l * koff2 + koff1 * kon2 * i + koff1 * koff2;
        if kf <= 0.0 || kf_ks <= 0.0 {
            return None;
        }
        let ks = kf_ks / kf;
        let z_s = -ks * t;
        let term1 = (koff2 / kf_ks) * (-z_s.exp_m1());
        let z_diff = -s * t;
        let phi1 = if z_diff.abs() < 1e-7 { 1.0 + z_diff * (0.5 + z_diff / 6.0) } else { z_diff.exp_m1() / z_diff };
        let term2 = (1.0 - koff2 / kf) * z_s.exp() * t * phi1;
        let y = bmax * kon1 * l * (term1 + term2);
        y.is_finite().then_some(y)
    }

    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (_, ymax) = data_y_min_max(points);
        vec![ymax.max(1.0), 1e6, 0.1, 1e6, 0.1, 1e-8, 1e-8]
    }

    fn default_bounds(&self) -> Vec<(Option<f64>, Option<f64>)> {
        vec![(Some(0.0), None), (Some(0.0), None), (Some(0.0), None), (Some(0.0), None), (Some(1e-12), None), (Some(0.0), None), (Some(0.0), None)]
    }
}

#[cfg(test)]
mod tests {
    use super::{PHARMA_EQUATION_IDS, resolve_pharma_model};

    /// Display names are written in CurveBook's own words (AGENTS.md › clean-room: no copied
    /// equation wording), so none uses the "Family -- Variant" separator style.
    #[test]
    fn display_names_use_own_wording() {
        for id in PHARMA_EQUATION_IDS {
            let model = resolve_pharma_model(id).unwrap_or_else(|| panic!("{id} does not resolve"));
            assert!(!model.name().contains(" -- "), "{id} uses the ` -- ` separator: {}", model.name());
        }
    }

    #[test]
    fn renamed_equations_keep_their_ids() {
        let expected = [
            ("log_agonist_vs_response_4pl", "log(agonist) vs. response, variable Hill slope (four parameters)"),
            ("log_agonist_vs_response_normalized_var", "log(agonist) vs. normalized response, variable Hill slope"),
            ("log_inhibitor_vs_response_4pl", "log(inhibitor) vs. response, variable Hill slope (four parameters)"),
            ("log_inhibitor_vs_response_normalized_var", "log(inhibitor) vs. normalized response, variable Hill slope"),
            ("agonist_vs_response_4pl", "[Agonist] vs. response, variable Hill slope (four parameters)"),
            ("inhibitor_vs_response_4pl", "[Inhibitor] vs. response, variable Hill slope (four parameters)"),
            ("one_site_specific_binding", "One-site specific binding"),
            ("one_site_specific_hill", "One-site specific binding with Hill slope"),
            ("one_site_total_and_ns", "One-site total and nonspecific binding"),
            ("one_site_fit_logic50", "One-site competitive binding (logIC50)"),
            ("operational_model_agonist", "Black\u{2013}Leff operational model, partial agonist"),
            ("two_sites_specific_binding", "Two-site specific binding"),
            ("one_site_fit_ki", "One-site competitive binding, Ki by Cheng\u{2013}Prusoff"),
            ("two_sites_fit_ki", "Two-site competitive binding (Ki)"),
            ("dissociation_one_phase", "One-phase exponential dissociation"),
            ("association_kinetics_two_conc", "Association kinetics at multiple radioligand concentrations"),
        ];
        for (id, name) in expected {
            let model = resolve_pharma_model(id).unwrap_or_else(|| panic!("{id} does not resolve"));
            assert_eq!(model.id(), id);
            assert_eq!(model.name(), name);
        }
    }
}
