//! Pharmacology, Dose–Response, Receptor Binding, and Enzyme Kinetics built-in equations.
//!
//! Includes analytical gradients, smart data-driven initial parameter estimates, default
//! domain bounds, and derived scalar transformations (`EC50`, `IC50`, `Span`, `Ki`, `Km`).

use crate::library::{ParametricModel, data_x_min_max, data_y_min_max, estimate_x_at_ymid};
use crate::types::{DataPoint, DerivedScalar, EquationCategory};

/// Canonical IDs exposed by the pharmacology / biochemistry equation catalog.
pub const PHARMA_EQUATION_IDS: &[&str] = &[
    // Dose-Response Stimulation
    "log_agonist_vs_response_3pl",
    "log_agonist_vs_response_4pl",
    "log_agonist_vs_response_normalized",
    "log_agonist_vs_response_normalized_var",
    "agonist_vs_response_3pl",
    "agonist_vs_response_4pl",
    "log_agonist_vs_response_5pl",
    "bell_shaped_dose_response",
    "biphasic_dose_response",
    "log_agonist_vs_response_ecf",
    "ec50_shift_gaddum_schild",
    "allosteric_ec50_shift",
    "operational_model_agonist",
    // Dose-Response Inhibition
    "log_inhibitor_vs_response_3pl",
    "log_inhibitor_vs_response_4pl",
    "log_inhibitor_vs_response_normalized",
    "log_inhibitor_vs_response_normalized_var",
    "inhibitor_vs_response_3pl",
    "inhibitor_vs_response_4pl",
    "absolute_ic50_4pl",
    // Receptor Binding
    "one_site_specific_binding",
    "one_site_specific_hill",
    "one_site_total_and_ns",
    "one_site_total_and_ns_global",
    "one_site_specific_ligand_depletion",
    "allosteric_modulator_saturation",
    "homologous_one_site_competition",
    "two_sites_specific_binding",
    "one_site_fit_ki",
    "one_site_fit_logic50",
    "two_sites_fit_ki",
    "dissociation_one_phase",
    "association_kinetics_one_conc",
    "association_kinetics_two_conc",
    "assoc_then_dissoc",
    "motulsky_mahan_kinetics",
    // Enzyme Kinetics
    "michaelis_menten",
    "kcat_enzyme_kinetics",
    "allosteric_sigmoidal_kinetics",
    "substrate_inhibition",
    "competitive_inhibition",
    "noncompetitive_inhibition",
    "uncompetitive_inhibition",
    "mixed_model_inhibition",
    "morrison_tight_binding_ki",
];

fn names(slice: &[&str]) -> Vec<String> {
    slice.iter().map(|s| (*s).to_owned()).collect()
}

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
    match norm_id {
        "log_agonist_vs_response_3pl" | "log_agonist_vs_response" | "ec50_3pl" => Some(Box::new(LogDoseResponse {
            id: "log_agonist_vs_response_3pl",
            name: "log(agonist) vs. response (three parameters)",
            category: EquationCategory::DoseResponseStimulation,
            variable_slope: false,
            is_inhibition: false,
            normalized: false,
            params: names(&["Bottom", "Top", "LogEC50"]),
        })),
        "log_agonist_vs_response_4pl" | "log_agonist_vs_response_variable_slope" | "4pl" => Some(Box::new(LogDoseResponse {
            id: "log_agonist_vs_response_4pl",
            name: "log(agonist) vs. response, variable Hill slope (four parameters)",
            category: EquationCategory::DoseResponseStimulation,
            variable_slope: true,
            is_inhibition: false,
            normalized: false,
            params: names(&["Bottom", "Top", "LogEC50", "HillSlope"]),
        })),
        "log_agonist_vs_response_normalized" => Some(Box::new(LogDoseResponse {
            id: "log_agonist_vs_response_normalized",
            name: "log(agonist) vs. normalized response",
            category: EquationCategory::DoseResponseStimulation,
            variable_slope: false,
            is_inhibition: false,
            normalized: true,
            params: names(&["LogEC50"]),
        })),
        "log_agonist_vs_response_normalized_var" => Some(Box::new(LogDoseResponse {
            id: "log_agonist_vs_response_normalized_var",
            name: "log(agonist) vs. normalized response, variable Hill slope",
            category: EquationCategory::DoseResponseStimulation,
            variable_slope: true,
            is_inhibition: false,
            normalized: true,
            params: names(&["LogEC50", "HillSlope"]),
        })),
        "log_inhibitor_vs_response_3pl" | "log_inhibitor_vs_response" | "ic50_3pl" => Some(Box::new(LogDoseResponse {
            id: "log_inhibitor_vs_response_3pl",
            name: "log(inhibitor) vs. response (three parameters)",
            category: EquationCategory::DoseResponseInhibition,
            variable_slope: false,
            is_inhibition: true,
            normalized: false,
            params: names(&["Bottom", "Top", "LogIC50"]),
        })),
        "log_inhibitor_vs_response_4pl" | "log_inhibitor_vs_response_variable_slope" => Some(Box::new(LogDoseResponse {
            id: "log_inhibitor_vs_response_4pl",
            name: "log(inhibitor) vs. response, variable Hill slope (four parameters)",
            category: EquationCategory::DoseResponseInhibition,
            variable_slope: true,
            is_inhibition: true,
            normalized: false,
            params: names(&["Bottom", "Top", "LogIC50", "HillSlope"]),
        })),
        "log_inhibitor_vs_response_normalized" => Some(Box::new(LogDoseResponse {
            id: "log_inhibitor_vs_response_normalized",
            name: "log(inhibitor) vs. normalized response",
            category: EquationCategory::DoseResponseInhibition,
            variable_slope: false,
            is_inhibition: true,
            normalized: true,
            params: names(&["LogIC50"]),
        })),
        "log_inhibitor_vs_response_normalized_var" => Some(Box::new(LogDoseResponse {
            id: "log_inhibitor_vs_response_normalized_var",
            name: "log(inhibitor) vs. normalized response, variable Hill slope",
            category: EquationCategory::DoseResponseInhibition,
            variable_slope: true,
            is_inhibition: true,
            normalized: true,
            params: names(&["LogIC50", "HillSlope"]),
        })),
        "agonist_vs_response_3pl" | "agonist_vs_response" => Some(Box::new(LinearDoseResponse {
            id: "agonist_vs_response_3pl",
            name: "[Agonist] vs. response (three parameters)",
            category: EquationCategory::DoseResponseStimulation,
            variable_slope: false,
            is_inhibition: false,
            params: names(&["Bottom", "Top", "EC50"]),
        })),
        "agonist_vs_response_4pl" => Some(Box::new(LinearDoseResponse {
            id: "agonist_vs_response_4pl",
            name: "[Agonist] vs. response, variable Hill slope (four parameters)",
            category: EquationCategory::DoseResponseStimulation,
            variable_slope: true,
            is_inhibition: false,
            params: names(&["Bottom", "Top", "EC50", "HillSlope"]),
        })),
        "inhibitor_vs_response_3pl" | "inhibitor_vs_response" => Some(Box::new(LinearDoseResponse {
            id: "inhibitor_vs_response_3pl",
            name: "[Inhibitor] vs. response (three parameters)",
            category: EquationCategory::DoseResponseInhibition,
            variable_slope: false,
            is_inhibition: true,
            params: names(&["Bottom", "Top", "IC50"]),
        })),
        "inhibitor_vs_response_4pl" => Some(Box::new(LinearDoseResponse {
            id: "inhibitor_vs_response_4pl",
            name: "[Inhibitor] vs. response, variable Hill slope (four parameters)",
            category: EquationCategory::DoseResponseInhibition,
            variable_slope: true,
            is_inhibition: true,
            params: names(&["Bottom", "Top", "IC50", "HillSlope"]),
        })),
        "log_agonist_vs_response_5pl" | "5pl" => Some(Box::new(RichardsFivePl { params: names(&["Bottom", "Top", "LogXb", "HillSlope", "S"]) })),
        "bell_shaped_dose_response" => {
            Some(Box::new(BellShapedDoseResponse { params: names(&["Plateau1", "Plateau2", "Dip", "LogEC50_1", "LogEC50_2", "nH1", "nH2"]) }))
        }
        "biphasic_dose_response" => {
            Some(Box::new(BiphasicDoseResponse { params: names(&["Bottom", "Top", "Frac", "LogEC50_1", "LogEC50_2", "nH1", "nH2"]) }))
        }
        "log_agonist_vs_response_ecf" | "ecf" => Some(Box::new(DoseResponseEcF { params: names(&["Bottom", "Top", "LogECF", "HillSlope", "F"]) })),
        "absolute_ic50_4pl" => Some(Box::new(AbsoluteIc50 { params: names(&["Bottom", "Top", "LogIC50", "HillSlope", "Baseline"]) })),
        "ec50_shift_gaddum_schild" | "gaddum_schild" => {
            Some(Box::new(GaddumSchild { params: names(&["Bottom", "Top", "LogEC50", "HillSlope", "pA2", "SchildSlope", "AntagConc"]) }))
        }
        "allosteric_ec50_shift" => {
            Some(Box::new(AllostericEc50Shift { params: names(&["Bottom", "Top", "LogEC50", "HillSlope", "Kb", "alpha", "ModConc"]) }))
        }
        "operational_model_agonist" => Some(Box::new(OperationalAgonism { params: names(&["Basal", "Emax", "LogKA", "LogTau", "n"]) })),
        "one_site_specific_binding" => Some(Box::new(OneSiteBinding {
            id: "one_site_specific_binding",
            name: "One-site specific binding",
            with_hill: false,
            with_ns: false,
            params: names(&["Bmax", "Kd"]),
        })),
        "one_site_specific_hill" => Some(Box::new(OneSiteBinding {
            id: "one_site_specific_hill",
            name: "One-site specific binding with Hill slope",
            with_hill: true,
            with_ns: false,
            params: names(&["Bmax", "Kd", "h"]),
        })),
        "one_site_total_and_ns" => Some(Box::new(OneSiteBinding {
            id: "one_site_total_and_ns",
            name: "One-site total and nonspecific binding",
            with_hill: false,
            with_ns: true,
            params: names(&["Bmax", "Kd", "NS", "Background"]),
        })),
        "one_site_total_and_ns_global" => Some(Box::new(OneSiteTotalAndNsGlobal { params: names(&["Bmax", "Kd", "NS", "Background"]) })),
        "one_site_specific_ligand_depletion" => Some(Box::new(OneSiteSpecificLigandDepletion { params: names(&["Rt", "Kd", "NS"]) })),
        "allosteric_modulator_saturation" => {
            Some(Box::new(AllostericModulatorSaturation { params: names(&["Bmax", "Kd", "alpha", "Kb", "ModConc"]) }))
        }
        "homologous_one_site_competition" => Some(Box::new(HomologousOneSiteCompetition { params: names(&["Bmax", "Kd", "NS", "HotConc"]) })),
        "two_sites_specific_binding" => Some(Box::new(TwoSiteBinding { params: names(&["BmaxHi", "KdHi", "BmaxLo", "KdLo"]) })),
        "one_site_fit_ki" => Some(Box::new(CompetitionOneSiteKi { params: names(&["Top", "Bottom", "LogKi", "HotConc_nM", "HotKd_nM"]) })),
        "one_site_fit_logic50" => Some(Box::new(LogDoseResponse {
            id: "one_site_fit_logic50",
            name: "One-site competitive binding (logIC50)",
            category: EquationCategory::ReceptorBinding,
            variable_slope: false,
            is_inhibition: true,
            normalized: false,
            params: names(&["Bottom", "Top", "LogIC50"]),
        })),
        "two_sites_fit_ki" => {
            Some(Box::new(CompetitionTwoSiteKi { params: names(&["Top", "Bottom", "FractionHi", "LogKi_1", "LogKi_2", "HotConc_nM", "HotKd_nM"]) }))
        }
        "dissociation_one_phase" => Some(Box::new(DissociationOnePhase { params: names(&["Y0", "NS", "Koff"]) })),
        "association_kinetics_one_conc" => Some(Box::new(AssociationOneConc { params: names(&["Ymax", "Kobs"]) })),
        "association_kinetics_two_conc" => Some(Box::new(AssociationTwoConc { params: names(&["Bmax", "Kon", "Koff", "LigandConc"]) })),
        "assoc_then_dissoc" => Some(Box::new(AssocThenDissoc { params: names(&["Bmax", "Kon", "Koff", "LigandConc", "Time0"]) })),
        "motulsky_mahan_kinetics" => Some(Box::new(MotulskyMahanKinetics { params: names(&["Bmax", "kon1", "koff1", "kon2", "koff2", "L", "I"]) })),
        "michaelis_menten" => Some(Box::new(MichaelisMenten { kcat_mode: false, params: names(&["Vmax", "Km"]) })),
        "kcat_enzyme_kinetics" | "kcat" => Some(Box::new(MichaelisMenten { kcat_mode: true, params: names(&["kcat", "Km", "Et"]) })),
        "allosteric_sigmoidal_kinetics" | "allosteric_sigmoidal" => Some(Box::new(AllostericSigmoidal { params: names(&["Vmax", "Khalf", "h"]) })),
        "substrate_inhibition" => Some(Box::new(SubstrateInhibition { params: names(&["Vmax", "Km", "Ki"]) })),
        "competitive_inhibition" => Some(Box::new(EnzymeInhibition {
            id: "competitive_inhibition",
            name: "Competitive inhibition",
            mode: EnzymeInhibMode::Competitive,
            params: names(&["Vmax", "Km", "Ki", "I"]),
        })),
        "noncompetitive_inhibition" => Some(Box::new(EnzymeInhibition {
            id: "noncompetitive_inhibition",
            name: "Noncompetitive inhibition",
            mode: EnzymeInhibMode::Noncompetitive,
            params: names(&["Vmax", "Km", "Ki", "I"]),
        })),
        "uncompetitive_inhibition" => Some(Box::new(EnzymeInhibition {
            id: "uncompetitive_inhibition",
            name: "Uncompetitive inhibition",
            mode: EnzymeInhibMode::Uncompetitive,
            params: names(&["Vmax", "Km", "alphaKi", "I"]),
        })),
        "mixed_model_inhibition" => Some(Box::new(EnzymeInhibition {
            id: "mixed_model_inhibition",
            name: "Mixed model inhibition",
            mode: EnzymeInhibMode::Mixed,
            params: names(&["Vmax", "Km", "Ki", "Alpha", "I"]),
        })),
        "morrison_tight_binding_ki" | "morrison" => Some(Box::new(MorrisonKi { params: names(&["Vo", "Et", "Ki", "S", "Km"]) })),
        _ => None,
    }
}

struct LogDoseResponse {
    id: &'static str,
    name: &'static str,
    category: EquationCategory,
    variable_slope: bool,
    is_inhibition: bool,
    normalized: bool,
    params: Vec<String>,
}

impl ParametricModel for LogDoseResponse {
    fn id(&self) -> &str {
        self.id
    }
    fn name(&self) -> &str {
        self.name
    }
    fn category(&self) -> EquationCategory {
        self.category
    }
    fn param_names(&self) -> &[String] {
        &self.params
    }
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

struct LinearDoseResponse {
    id: &'static str,
    name: &'static str,
    category: EquationCategory,
    variable_slope: bool,
    is_inhibition: bool,
    params: Vec<String>,
}

impl ParametricModel for LinearDoseResponse {
    fn id(&self) -> &str {
        self.id
    }
    fn name(&self) -> &str {
        self.name
    }
    fn category(&self) -> EquationCategory {
        self.category
    }
    fn param_names(&self) -> &[String] {
        &self.params
    }
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
        if !c.is_finite() || c < 0.0 || params.len() < self.params.len() || out.len() < self.params.len() {
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
        out[..self.params.len()].iter().all(|v| v.is_finite())
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

struct RichardsFivePl {
    params: Vec<String>,
}

impl ParametricModel for RichardsFivePl {
    fn id(&self) -> &str {
        "log_agonist_vs_response_5pl"
    }
    fn name(&self) -> &str {
        "Asymmetric (five parameters) log(agonist) vs. response"
    }
    fn category(&self) -> EquationCategory {
        EquationCategory::DoseResponseStimulation
    }
    fn param_names(&self) -> &[String] {
        &self.params
    }
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
        if s > 0.0 && hill.abs() > 1e-12 {
            let log_ec50 = log_xb - ((2.0_f64.powf(1.0 / s) - 1.0).log10()) / hill;
            out.push(DerivedScalar { name: "LogEC50".to_owned(), value: log_ec50, units: None });
            if let Some(ec50) = safe_pow10(log_ec50) {
                out.push(DerivedScalar { name: "EC50".to_owned(), value: ec50, units: None });
            }
        }
        out
    }
}

struct BellShapedDoseResponse {
    params: Vec<String>,
}

impl ParametricModel for BellShapedDoseResponse {
    fn id(&self) -> &str {
        "bell_shaped_dose_response"
    }
    fn name(&self) -> &str {
        "Bell-shaped dose-response"
    }
    fn category(&self) -> EquationCategory {
        EquationCategory::DoseResponseStimulation
    }
    fn param_names(&self) -> &[String] {
        &self.params
    }
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

struct BiphasicDoseResponse {
    params: Vec<String>,
}

impl ParametricModel for BiphasicDoseResponse {
    fn id(&self) -> &str {
        "biphasic_dose_response"
    }
    fn name(&self) -> &str {
        "Biphasic dose-response"
    }
    fn category(&self) -> EquationCategory {
        EquationCategory::DoseResponseStimulation
    }
    fn param_names(&self) -> &[String] {
        &self.params
    }
    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let x0 = x.first().copied()?;
        let (bottom, top, frac, l1, l2, n1, n2) = (params[0], params[1], params[2], params[3], params[4], params[5], params[6]);
        let (s1, _) = sigmoid_from_log_x(x0, l1, -n1)?;
        let (s2, _) = sigmoid_from_log_x(x0, l2, -n2)?;
        Some(bottom + (top - bottom) * (frac * s1 + (1.0 - frac) * s2))
    }
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (ymin, ymax) = data_y_min_max(points);
        let (xmin, xmax) = data_x_min_max(points);
        let span = (xmax - xmin).max(1.0);
        vec![ymin, ymax, 0.5, xmin + 0.3 * span, xmin + 0.7 * span, 1.0, 1.0]
    }
    fn default_bounds(&self) -> Vec<(Option<f64>, Option<f64>)> {
        vec![(None, None), (None, None), (Some(0.0), Some(1.0)), (None, None), (None, None), (None, None), (None, None)]
    }
}

struct DoseResponseEcF {
    params: Vec<String>,
}

impl ParametricModel for DoseResponseEcF {
    fn id(&self) -> &str {
        "log_agonist_vs_response_ecf"
    }
    fn name(&self) -> &str {
        "ECanything (log(agonist) vs. response)"
    }
    fn category(&self) -> EquationCategory {
        EquationCategory::DoseResponseStimulation
    }
    fn param_names(&self) -> &[String] {
        &self.params
    }
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

struct AbsoluteIc50 {
    params: Vec<String>,
}

impl ParametricModel for AbsoluteIc50 {
    fn id(&self) -> &str {
        "absolute_ic50_4pl"
    }
    fn name(&self) -> &str {
        "Absolute IC50, four parameter"
    }
    fn category(&self) -> EquationCategory {
        EquationCategory::DoseResponseInhibition
    }
    fn param_names(&self) -> &[String] {
        &self.params
    }
    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let x0 = x.first().copied()?;
        let (bottom, top, log_abs_ic50, hill, baseline) = (params[0], params[1], params[2], params[3], params[4]);
        let fifty = 0.5 * baseline;
        if top <= fifty || bottom >= fifty || (top - bottom).abs() < 1e-15 {
            return None;
        }
        let log_ic50 = log_abs_ic50 - ((top - fifty) / (fifty - bottom)).log10() / hill;
        let (sig, _) = sigmoid_from_log_x(x0, log_ic50, -hill)?;
        Some(bottom + (top - bottom) * sig)
    }
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (ymin, ymax) = data_y_min_max(points);
        vec![ymin, ymax, estimate_x_at_ymid(points), 1.0, ymax]
    }
}

struct GaddumSchild {
    params: Vec<String>,
}

impl ParametricModel for GaddumSchild {
    fn id(&self) -> &str {
        "ec50_shift_gaddum_schild"
    }
    fn name(&self) -> &str {
        "Gaddum/Schild EC50 shift"
    }
    fn category(&self) -> EquationCategory {
        EquationCategory::DoseResponseStimulation
    }
    fn param_names(&self) -> &[String] {
        &self.params
    }
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

struct AllostericEc50Shift {
    params: Vec<String>,
}

impl ParametricModel for AllostericEc50Shift {
    fn id(&self) -> &str {
        "allosteric_ec50_shift"
    }
    fn name(&self) -> &str {
        "Allosteric EC50 shift"
    }
    fn category(&self) -> EquationCategory {
        EquationCategory::DoseResponseStimulation
    }
    fn param_names(&self) -> &[String] {
        &self.params
    }
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

struct OperationalAgonism {
    params: Vec<String>,
}

impl ParametricModel for OperationalAgonism {
    fn id(&self) -> &str {
        "operational_model_agonist"
    }
    fn name(&self) -> &str {
        "Black–Leff operational model, partial agonist"
    }
    fn category(&self) -> EquationCategory {
        EquationCategory::DoseResponseStimulation
    }
    fn param_names(&self) -> &[String] {
        &self.params
    }
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
        let denom = operate + ((a + ka) / den).powf(n);
        let term = if operate.is_finite() && denom.is_finite() {
            operate / (1.0 + operate)
        } else {
            return None;
        };
        let _ = denom;
        Some(basal + (emax - basal) * term)
    }
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (ymin, ymax) = data_y_min_max(points);
        vec![ymin, ymax, estimate_x_at_ymid(points), 0.5, 1.0]
    }
}

struct OneSiteBinding {
    id: &'static str,
    name: &'static str,
    with_hill: bool,
    with_ns: bool,
    params: Vec<String>,
}

impl ParametricModel for OneSiteBinding {
    fn id(&self) -> &str {
        self.id
    }
    fn name(&self) -> &str {
        self.name
    }
    fn category(&self) -> EquationCategory {
        EquationCategory::ReceptorBinding
    }
    fn param_names(&self) -> &[String] {
        &self.params
    }
    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let c = x.first().copied()?;
        if c < 0.0 || !c.is_finite() {
            return None;
        }
        let (bmax, kd) = (params[0], params[1]);
        if kd <= 0.0 {
            return None;
        }
        if self.with_hill {
            let h = params[2];
            if c == 0.0 {
                return Some(0.0);
            }
            let ch = c.powf(h);
            let kdh = kd.powf(h);
            return Some(bmax * ch / (kdh + ch));
        }
        let spec = bmax * c / (kd + c);
        if self.with_ns { Some(spec + params[2] * c + params[3]) } else { Some(spec) }
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

struct TwoSiteBinding {
    params: Vec<String>,
}

impl ParametricModel for TwoSiteBinding {
    fn id(&self) -> &str {
        "two_sites_specific_binding"
    }
    fn name(&self) -> &str {
        "Two-site specific binding"
    }
    fn category(&self) -> EquationCategory {
        EquationCategory::ReceptorBinding
    }
    fn param_names(&self) -> &[String] {
        &self.params
    }
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

struct CompetitionOneSiteKi {
    params: Vec<String>,
}

impl ParametricModel for CompetitionOneSiteKi {
    fn id(&self) -> &str {
        "one_site_fit_ki"
    }
    fn name(&self) -> &str {
        "One-site competitive binding, Ki by Cheng–Prusoff"
    }
    fn category(&self) -> EquationCategory {
        EquationCategory::ReceptorBinding
    }
    fn param_names(&self) -> &[String] {
        &self.params
    }
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

struct CompetitionTwoSiteKi {
    params: Vec<String>,
}

impl ParametricModel for CompetitionTwoSiteKi {
    fn id(&self) -> &str {
        "two_sites_fit_ki"
    }
    fn name(&self) -> &str {
        "Two-site competitive binding (Ki)"
    }
    fn category(&self) -> EquationCategory {
        EquationCategory::ReceptorBinding
    }
    fn param_names(&self) -> &[String] {
        &self.params
    }
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

struct DissociationOnePhase {
    params: Vec<String>,
}

impl ParametricModel for DissociationOnePhase {
    fn id(&self) -> &str {
        "dissociation_one_phase"
    }
    fn name(&self) -> &str {
        "One-phase exponential dissociation"
    }
    fn category(&self) -> EquationCategory {
        EquationCategory::ReceptorBinding
    }
    fn param_names(&self) -> &[String] {
        &self.params
    }
    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let t = x.first().copied()?;
        let (y0, ns, koff) = (params[0], params[1], params[2]);
        let arg = -koff * t;
        if arg > 700.0 {
            return None;
        }
        Some((y0 - ns) * arg.exp() + ns)
    }
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (ymin, ymax) = data_y_min_max(points);
        let (xmin, xmax) = data_x_min_max(points);
        let span = (xmax - xmin).max(1.0);
        vec![ymax, ymin, 1.0 / span]
    }
}

struct AssociationOneConc {
    params: Vec<String>,
}

impl ParametricModel for AssociationOneConc {
    fn id(&self) -> &str {
        "association_kinetics_one_conc"
    }
    fn name(&self) -> &str {
        "One phase association (one concentration of hot)"
    }
    fn category(&self) -> EquationCategory {
        EquationCategory::ReceptorBinding
    }
    fn param_names(&self) -> &[String] {
        &self.params
    }
    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let t = x.first().copied()?;
        let (ymax, kobs) = (params[0], params[1]);
        let arg = -kobs * t;
        if arg > 700.0 {
            return None;
        }
        Some(ymax * (1.0 - arg.exp()))
    }
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (_, ymax) = data_y_min_max(points);
        let (xmin, xmax) = data_x_min_max(points);
        vec![ymax, 2.0 / (xmax - xmin).max(1.0)]
    }
}

struct AssociationTwoConc {
    params: Vec<String>,
}

impl ParametricModel for AssociationTwoConc {
    fn id(&self) -> &str {
        "association_kinetics_two_conc"
    }
    fn name(&self) -> &str {
        "Association kinetics at multiple radioligand concentrations"
    }
    fn category(&self) -> EquationCategory {
        EquationCategory::ReceptorBinding
    }
    fn param_names(&self) -> &[String] {
        &self.params
    }
    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let t = x.first().copied()?;
        let (bmax, kon, koff, conc) = (params[0], params[1], params[2], params[3]);
        let kobs = kon * conc + koff;
        if kobs <= 0.0 || (-kobs * t) > 700.0 {
            return None;
        }
        let eq = bmax * kon * conc / kobs;
        Some(eq * (1.0 - (-kobs * t).exp()))
    }
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (_, ymax) = data_y_min_max(points);
        vec![ymax, 1e6, 0.01, 1e-8]
    }
}

struct AssocThenDissoc {
    params: Vec<String>,
}

impl ParametricModel for AssocThenDissoc {
    fn id(&self) -> &str {
        "assoc_then_dissoc"
    }
    fn name(&self) -> &str {
        "Association then dissociation"
    }
    fn category(&self) -> EquationCategory {
        EquationCategory::ReceptorBinding
    }
    fn param_names(&self) -> &[String] {
        &self.params
    }
    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let t = x.first().copied()?;
        let (bmax, kon, koff, conc, t0) = (params[0], params[1], params[2], params[3], params[4]);
        let kobs = kon * conc + koff;
        if kobs <= 0.0 {
            return None;
        }
        let eq = bmax * kon * conc / kobs;
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
    }
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (_, ymax) = data_y_min_max(points);
        let (xmin, xmax) = data_x_min_max(points);
        vec![ymax, 1e6, 0.05, 1e-8, 0.5 * (xmin + xmax)]
    }
}

struct MichaelisMenten {
    kcat_mode: bool,
    params: Vec<String>,
}

impl ParametricModel for MichaelisMenten {
    fn id(&self) -> &str {
        if self.kcat_mode { "kcat_enzyme_kinetics" } else { "michaelis_menten" }
    }
    fn name(&self) -> &str {
        if self.kcat_mode { "kcat" } else { "Michaelis-Menten" }
    }
    fn category(&self) -> EquationCategory {
        EquationCategory::EnzymeKinetics
    }
    fn param_names(&self) -> &[String] {
        &self.params
    }
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
        let km = estimate_x_at_ymid(points).abs().max(1e-3);
        if self.kcat_mode { vec![ymax.max(1.0), km, 1.0] } else { vec![ymax.max(1.0), km] }
    }
    fn default_bounds(&self) -> Vec<(Option<f64>, Option<f64>)> {
        if self.kcat_mode { vec![(Some(0.0), None), (Some(1e-30), None), (Some(0.0), None)] } else { vec![(Some(0.0), None), (Some(1e-30), None)] }
    }
}

struct AllostericSigmoidal {
    params: Vec<String>,
}

impl ParametricModel for AllostericSigmoidal {
    fn id(&self) -> &str {
        "allosteric_sigmoidal_kinetics"
    }
    fn name(&self) -> &str {
        "Allosteric sigmoidal"
    }
    fn category(&self) -> EquationCategory {
        EquationCategory::EnzymeKinetics
    }
    fn param_names(&self) -> &[String] {
        &self.params
    }
    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let s = x.first().copied()?;
        let (vmax, khalf, h) = (params[0], params[1], params[2]);
        if s < 0.0 || khalf <= 0.0 {
            return None;
        }
        if s == 0.0 {
            return Some(0.0);
        }
        let sh = s.powf(h);
        let kh = khalf.powf(h);
        if !sh.is_finite() || !kh.is_finite() || (kh + sh) == 0.0 {
            return None;
        }
        Some(vmax * sh / (kh + sh))
    }
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (_, ymax) = data_y_min_max(points);
        vec![ymax.max(1.0), estimate_x_at_ymid(points).abs().max(1e-3), 1.5]
    }
}

struct SubstrateInhibition {
    params: Vec<String>,
}

impl ParametricModel for SubstrateInhibition {
    fn id(&self) -> &str {
        "substrate_inhibition"
    }
    fn name(&self) -> &str {
        "Substrate inhibition"
    }
    fn category(&self) -> EquationCategory {
        EquationCategory::EnzymeKinetics
    }
    fn param_names(&self) -> &[String] {
        &self.params
    }
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

struct EnzymeInhibition {
    id: &'static str,
    name: &'static str,
    mode: EnzymeInhibMode,
    params: Vec<String>,
}

impl ParametricModel for EnzymeInhibition {
    fn id(&self) -> &str {
        self.id
    }
    fn name(&self) -> &str {
        self.name
    }
    fn category(&self) -> EquationCategory {
        EquationCategory::EnzymeKinetics
    }
    fn param_names(&self) -> &[String] {
        &self.params
    }
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

struct MorrisonKi {
    params: Vec<String>,
}

impl ParametricModel for MorrisonKi {
    fn id(&self) -> &str {
        "morrison_tight_binding_ki"
    }
    fn name(&self) -> &str {
        "Morrison tight-binding Ki"
    }
    fn category(&self) -> EquationCategory {
        EquationCategory::EnzymeKinetics
    }
    fn param_names(&self) -> &[String] {
        &self.params
    }
    fn eval(&self, x: &[f64], params: &[f64], _dataset_index: usize) -> Option<f64> {
        let i_conc = x.first().copied()?;
        let (vo, et, ki, s_conc, km) = (params[0], params[1], params[2], params[3], params[4]);
        if i_conc < 0.0 || et <= 0.0 || ki <= 0.0 || km <= 0.0 {
            return None;
        }
        let ki_app = ki * (1.0 + s_conc / km);
        let sum = et + i_conc + ki_app;
        let disc = (sum * sum - 4.0 * et * i_conc).max(0.0).sqrt();
        Some(vo * (1.0 - (sum - disc) / (2.0 * et)))
    }
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let (_, ymax) = data_y_min_max(points);
        vec![ymax.max(1.0), 1.0, 0.1, 1.0, 1.0]
    }
}

/// Global one-site total and nonspecific binding (`dataset 0` = Total, `dataset >= 1` = Nonspecific).
struct OneSiteTotalAndNsGlobal {
    params: Vec<String>,
}

impl ParametricModel for OneSiteTotalAndNsGlobal {
    fn id(&self) -> &str {
        "one_site_total_and_ns_global"
    }

    fn name(&self) -> &str {
        "One-site total and nonspecific binding (global)"
    }

    fn category(&self) -> EquationCategory {
        EquationCategory::ReceptorBinding
    }

    fn param_names(&self) -> &[String] {
        &self.params
    }

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
        let ds0: Vec<DataPoint> = points.iter().filter(|p| p.data_set_index == 0).cloned().collect();
        let ds1: Vec<DataPoint> = points.iter().filter(|p| p.data_set_index >= 1).cloned().collect();
        let primary = if ds0.is_empty() { points } else { &ds0 };
        let (ymin, ymax) = data_y_min_max(primary);
        let (_, xmax) = data_x_min_max(primary);
        let kd = estimate_x_at_ymid(primary).max(1e-6);
        let (ns, bg) = if ds1.len() >= 2 {
            let n = ds1.len() as f64;
            let mx = ds1.iter().map(|p| p.x0()).sum::<f64>() / n;
            let my = ds1.iter().map(|p| p.y).sum::<f64>() / n;
            let mut sxx = 0.0_f64;
            let mut sxy = 0.0_f64;
            for p in &ds1 {
                let dx = p.x0() - mx;
                sxx += dx * dx;
                sxy += dx * (p.y - my);
            }
            let slope = if sxx > 1e-30 { (sxy / sxx).max(0.0) } else { 0.0 };
            (slope, my - slope * mx)
        } else {
            (((ymax - ymin) / xmax.max(1e-12)) * 0.05, ymin)
        };
        let bmax = (ymax - bg - ns * xmax).max(ymax - ymin).max(1.0);
        vec![bmax, kd, ns, bg]
    }

    fn default_bounds(&self) -> Vec<(Option<f64>, Option<f64>)> {
        vec![(Some(0.0), None), (Some(1e-12), None), (Some(0.0), None), (None, None)]
    }
}

/// Specific saturation binding with ligand depletion (Swillens 1995 / Morrison quadratic root).
struct OneSiteSpecificLigandDepletion {
    params: Vec<String>,
}

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
    fn id(&self) -> &str {
        "one_site_specific_ligand_depletion"
    }

    fn name(&self) -> &str {
        "One-site specific binding with ligand depletion"
    }

    fn category(&self) -> EquationCategory {
        EquationCategory::ReceptorBinding
    }

    fn param_names(&self) -> &[String] {
        &self.params
    }

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

/// Allosteric ternary-complex saturation binding with modulator concentration `ModConc`.
struct AllostericModulatorSaturation {
    params: Vec<String>,
}

impl ParametricModel for AllostericModulatorSaturation {
    fn id(&self) -> &str {
        "allosteric_modulator_saturation"
    }

    fn name(&self) -> &str {
        "Allosteric modulator saturation binding"
    }

    fn category(&self) -> EquationCategory {
        EquationCategory::ReceptorBinding
    }

    fn param_names(&self) -> &[String] {
        &self.params
    }

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

/// Homologous one-site competitive binding (`Kd_cold == Kd_hot`, `X = log10[Cold]`).
struct HomologousOneSiteCompetition {
    params: Vec<String>,
}

impl ParametricModel for HomologousOneSiteCompetition {
    fn id(&self) -> &str {
        "homologous_one_site_competition"
    }

    fn name(&self) -> &str {
        "Homologous one-site competitive binding"
    }

    fn category(&self) -> EquationCategory {
        EquationCategory::ReceptorBinding
    }

    fn param_names(&self) -> &[String] {
        &self.params
    }

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
struct MotulskyMahanKinetics {
    params: Vec<String>,
}

impl ParametricModel for MotulskyMahanKinetics {
    fn id(&self) -> &str {
        "motulsky_mahan_kinetics"
    }

    fn name(&self) -> &str {
        "Motulsky-Mahan competitive binding kinetics"
    }

    fn category(&self) -> EquationCategory {
        EquationCategory::ReceptorBinding
    }

    fn param_names(&self) -> &[String] {
        &self.params
    }

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

    /// Display names are written in LumenCraft's own words (AGENTS.md › clean-room: no copied
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
