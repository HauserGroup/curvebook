//! Two-pane `InfoSheet` model: structured hookable laboratory constants on the left pane and
//! freeform Markdown experimental notes on the right pane (`docs/data-tables-and-transforms.md` §3).

use lumencraft_palette::Rgba8;

use crate::error::{DocError, validate_title};
use crate::ids::{FloatingNote, SheetId};

/// Standard laboratory metadata keys pre-seeded on new `InfoSheet` instances
/// (`docs/data-tables-and-transforms.md` §3).
pub const STANDARD_INFO_KEYS: [&str; 6] = ["Experiment Date", "Experiment ID", "Notebook ID", "Project", "Experimenter", "Protocol"];

/// Typed value stored in an [`InfoConstant`] row.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum ConstantValue {
    #[default]
    Empty,
    Number(f64),
    Text(String),
    /// ISO-8601 date string (`YYYY-MM-DD`).
    Date(String),
}

impl ConstantValue {
    /// Extract a finite scalar `f64` when this constant is hooked into a mathematical equation or
    /// normalization anchor (`Number(x)` or numeric `Text`).
    #[must_use]
    pub fn as_finite_f64(&self) -> Option<f64> {
        match self {
            Self::Number(v) if v.is_finite() => Some(*v),
            Self::Text(s) => s.trim().parse::<f64>().ok().filter(|v| v.is_finite()),
            Self::Empty | Self::Number(_) | Self::Date(_) => None,
        }
    }

    /// Format for inline text interpolation (`{{info:Name}}`).
    #[must_use]
    pub fn display_text(&self) -> String {
        match self {
            Self::Empty => String::new(),
            Self::Number(v) if v.is_finite() => format!("{v}"),
            Self::Number(_) => String::new(),
            Self::Text(s) | Self::Date(s) => s.clone(),
        }
    }
}

/// Structured key–value constant on an [`InfoSheet`] (`docs/data-tables-and-transforms.md` §3).
#[derive(Debug, Clone, PartialEq)]
pub struct InfoConstant {
    pub name: String,
    pub value: ConstantValue,
    pub units: Option<String>,
}

impl InfoConstant {
    pub fn new(name: impl Into<String>, value: ConstantValue, units: Option<String>) -> Result<Self, DocError> {
        let name = name.into();
        if name.trim().is_empty() {
            return Err(DocError::EmptyIdentifier);
        }
        validate_title(&name)?;
        if let Some(u) = &units {
            validate_title(u)?;
        }
        Ok(Self { name, value, units })
    }
}

/// Two-pane laboratory metadata and experimental notes sheet paired with (or shared across) one or
/// more [`crate::DataSheet`]s.
#[derive(Debug, Clone, PartialEq)]
pub struct InfoSheet {
    pub id: SheetId,
    pub title: String,
    pub linked_data_sheets: Vec<SheetId>,
    pub constants: Vec<InfoConstant>,
    pub notes_markdown: String,
    pub floating_notes: Vec<FloatingNote>,
    pub color_tag: Option<Rgba8>,
}

impl InfoSheet {
    /// Create a new `InfoSheet` pre-seeded with [`STANDARD_INFO_KEYS`].
    pub fn new_preseeded(id: impl Into<SheetId>, title: impl Into<String>) -> Result<Self, DocError> {
        let id = id.into();
        if id.as_str().trim().is_empty() {
            return Err(DocError::EmptyIdentifier);
        }
        let title = title.into();
        validate_title(&title)?;
        let constants =
            STANDARD_INFO_KEYS.iter().map(|&key| InfoConstant { name: key.to_owned(), value: ConstantValue::Empty, units: None }).collect();
        Ok(Self { id, title, linked_data_sheets: Vec::new(), constants, notes_markdown: String::new(), floating_notes: Vec::new(), color_tag: None })
    }

    /// Look up a constant by case-insensitive trimmed name.
    #[must_use]
    pub fn get_constant(&self, name: &str) -> Option<&InfoConstant> {
        let needle = name.trim();
        self.constants.iter().find(|c| c.name.trim().eq_ignore_ascii_case(needle))
    }

    /// Upsert a named constant (`info.setConstant`), updating in place if the key already exists
    /// or appending a new row otherwise.
    pub fn set_constant(&mut self, name: impl Into<String>, value: ConstantValue, units: Option<String>) -> Result<(), DocError> {
        let entry = InfoConstant::new(name, value, units)?;
        let needle = entry.name.trim();
        if let Some(existing) = self.constants.iter_mut().find(|c| c.name.trim().eq_ignore_ascii_case(needle)) {
            existing.value = entry.value;
            existing.units = entry.units;
        } else {
            self.constants.push(entry);
        }
        Ok(())
    }

    /// Remove a constant by case-insensitive name, returning `true` if one was removed.
    pub fn remove_constant(&mut self, name: &str) -> bool {
        let needle = name.trim();
        let before = self.constants.len();
        self.constants.retain(|c| !c.name.trim().eq_ignore_ascii_case(needle));
        self.constants.len() < before
    }
}
