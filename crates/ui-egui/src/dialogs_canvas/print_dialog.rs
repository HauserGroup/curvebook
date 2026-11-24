//! `Print...` setup and OS print-spool dialog state machine (`R5.4`).

use curvebook_doc::{ProjectDocument, SheetId};

use serde::{Deserialize, Serialize};

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
}
