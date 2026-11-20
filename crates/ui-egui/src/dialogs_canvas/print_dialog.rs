//! `Print...` setup and OS print-spool dialog state machine (`R5.4`).

use curvebook_doc::{ProjectDocument, SheetId};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::i18n::LocalePreset;

use super::parse_helpers::parse_dialog_num_with_locale;

/// Interactive state for the `Print...` setup and OS print-spool dialog (`R5.4`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrintDialogState {
    pub job: crate::PrintJobSpec,
    pub copies: u16,
    pub selected_printer: Option<String>,
    pub discovered_printers: Vec<String>,
    pub last_spool_bytes: usize,
    pub last_status: Option<String>,
    #[serde(default)]
    pub submit_to_os: bool,
}

impl Default for PrintDialogState {
    fn default() -> Self {
        Self {
            job: crate::PrintJobSpec::default(),
            copies: 1,
            selected_printer: None,
            discovered_printers: Vec::new(),
            last_spool_bytes: 0,
            last_status: None,
            submit_to_os: false,
        }
    }
}

impl PrintDialogState {
    /// Initialize dialog settings targeting `active_sheet` (falling back to the first graph or
    /// layout sheet in `doc`) and refresh discovered OS print queues.
    pub fn open_for_document(&mut self, doc: &ProjectDocument, active_sheet: Option<&SheetId>, discovered_printers: Vec<String>) {
        let resolved_id = active_sheet
            .filter(|id| doc.graph_sheet(id).is_some() || doc.layout_sheet(id).is_some())
            .cloned()
            .or_else(|| doc.layout_sheets.first().map(|ls| ls.id.clone()))
            .or_else(|| doc.graph_sheets.first().map(|gs| gs.id.clone()));

        if let Some(id) = &resolved_id {
            self.job.sheet_ids = vec![id.clone()];
            if let Some(ls) = doc.layout_sheet(id) {
                self.job.title = Some(ls.title.clone());
            } else if let Some(gs) = doc.graph_sheet(id) {
                self.job.title = Some(gs.title.clone());
            }
        } else {
            self.job.sheet_ids.clear();
            self.job.title = Some(doc.title.clone());
        }

        self.copies = self.copies.clamp(1, 999);
        self.submit_to_os = false;
        self.discovered_printers = discovered_printers;
        if let Some(sel) = &self.selected_printer
            && !self.discovered_printers.is_empty()
            && !self.discovered_printers.iter().any(|p| p == sel)
        {
            self.selected_printer = self.discovered_printers.first().cloned();
        }
        self.last_status = Some(match self.discovered_printers.len() {
            0 => String::from("Ready to spool PDF at 100% physical scale"),
            n => format!("{n} OS printer queue(s) discovered"),
        });
    }

    /// Update desired print copy count, clamping into `1..=999`.
    pub fn update_copies(&mut self, copies: u16) -> u16 {
        self.copies = copies.clamp(1, 999);
        self.copies
    }

    /// Select an OS printer queue name (or `None` for the OS default queue).
    pub fn select_printer(&mut self, printer: Option<String>) {
        self.selected_printer = printer.map(|p| p.trim().to_owned()).filter(|p| !p.is_empty());
    }

    /// Update print scale percentage, clamping finite values into `10.0..=400.0`.
    pub fn set_scale_percent(&mut self, scale_percent: f64) {
        if scale_percent.is_finite() {
            self.job.scale_percent = scale_percent.clamp(10.0, 400.0);
        }
    }

    /// Patch `Print` modal fields (`crop_marks`, `bleed_pt`, `copies`, `scale_percent`, `printer`, `title`).
    ///
    /// # Errors
    /// Returns an error when any numeric field is out of valid range or has an invalid type.
    /// Apply modal field updates from `ui.answer_dialog` / widget controls.
    pub fn apply_field_patch(&mut self, fields: &serde_json::Map<String, Value>) -> Result<(), String> {
        self.apply_field_patch_localized(fields, &LocalePreset::period_comma("en-US"))
    }

    /// Apply modal field updates using regional numeric parsing (`preset`).
    pub fn apply_field_patch_localized(&mut self, fields: &serde_json::Map<String, Value>, preset: &LocalePreset) -> Result<(), String> {
        for (key, val) in fields {
            match key.as_str() {
                "crop_marks" | "print.crop_marks" => {
                    self.job.crop_marks = val.as_bool().ok_or_else(|| String::from("`crop_marks` must be boolean"))?;
                }
                "bleed_pt" | "print.bleed_pt" => {
                    let bleed = parse_dialog_num_with_locale(val, key, preset)?;
                    if !(0.0..=144.0).contains(&bleed) {
                        return Err(format!("`bleed_pt` ({bleed}) must be within 0.0..=144.0"));
                    }
                    self.job.bleed_pt = bleed;
                }
                "copies" | "print.copies" => {
                    let raw = parse_dialog_num_with_locale(val, key, preset)?;
                    if raw < 1.0 || raw > 999.0 || (raw - raw.round()).abs() > 1e-9 {
                        return Err(format!("`copies` ({raw}) must be an integer within 1..=999"));
                    }
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    let count = raw.round() as u16;
                    self.update_copies(count);
                }
                "scale_percent" | "print.scale_percent" => {
                    let scale = parse_dialog_num_with_locale(val, key, preset)?;
                    if !(10.0..=400.0).contains(&scale) {
                        return Err(format!("`scale_percent` ({scale}) must be within 10.0..=400.0"));
                    }
                    self.set_scale_percent(scale);
                }
                "selected_printer" | "printer" | "print.printer" | "print.selected_printer" => {
                    if val.is_null() {
                        self.select_printer(None);
                    } else {
                        let s = val.as_str().ok_or_else(|| String::from("`printer` must be a string or null"))?;
                        self.select_printer(Some(s.to_owned()));
                    }
                }
                "title" | "print.title" => {
                    if val.is_null() {
                        self.job.title = None;
                    } else {
                        let s = val.as_str().ok_or_else(|| String::from("`title` must be a string or null"))?;
                        self.job.title = (!s.trim().is_empty()).then(|| s.trim().to_owned());
                    }
                }
                "submit_to_os" | "print.submit_to_os" => {
                    self.submit_to_os = val.as_bool().ok_or_else(|| String::from("`submit_to_os` must be boolean"))?;
                }
                other => return Err(format!("unknown Print field `{other}`")),
            }
        }
        Ok(())
    }
}
