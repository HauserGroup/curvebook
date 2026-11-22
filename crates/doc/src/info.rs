//! Two-pane `InfoSheet` model: structured hookable laboratory constants on the left pane and
//! freeform Markdown experimental notes on the right pane (`docs/implementation/data-tables-and-transforms.md` §3).

use curvebook_palette::Rgba8;

use crate::error::{DocError, validate_title};
use crate::ids::{FloatingNote, SheetId};

/// Standard laboratory metadata keys pre-seeded on new `InfoSheet` instances
/// (`docs/implementation/data-tables-and-transforms.md` §3).
pub const STANDARD_INFO_KEYS: [&str; 6] = ["Experiment Date", "Experiment ID", "Notebook ID", "Project", "Experimenter", "Protocol"];

/// Typed value stored in an [`InfoConstant`] row.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub enum ConstantValue {
    /// Unassigned metadata slot (for example a freshly pre-seeded [`STANDARD_INFO_KEYS`] row).
    #[default]
    Empty,
    /// Numeric experimental constant (for example radioligand concentration or assay temperature) directly usable by [`Self::as_finite_f64`].
    Number(f64),
    /// Free-form string metadata (also parsed by [`Self::as_finite_f64`] when its trimmed text is a finite decimal number).
    Text(String),
    /// ISO-8601 calendar date string (`YYYY-MM-DD`); never coerced to `f64` by [`Self::as_finite_f64`].
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

/// Structured key–value constant on an [`InfoSheet`] (`docs/implementation/data-tables-and-transforms.md` §3).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct InfoConstant {
    /// Non-empty constant key matched case-insensitively by [`InfoSheet::get_constant`] and `{{info:Name}}` hooks.
    pub name: String,
    /// Typed scalar, text, date, or empty value assigned to `name`.
    pub value: ConstantValue,
    /// Optional physical or concentration units label (for example `"nM"` or `"°C"`), or `None` when dimensionless/unspecified.
    pub units: Option<String>,
}

impl InfoConstant {
    /// Construct a validated `InfoConstant`, rejecting empty names, strings exceeding [`crate::MAX_TITLE_BYTES`], or non-finite numbers.
    pub fn new(name: impl Into<String>, value: ConstantValue, units: Option<String>) -> Result<Self, DocError> {
        let entry = Self { name: name.into(), value, units };
        entry.validate()?;
        Ok(entry)
    }

    /// Validate name, optional units title length, and finiteness of numeric constant values.
    pub fn validate(&self) -> Result<(), DocError> {
        if self.name.trim().is_empty() {
            return Err(DocError::EmptyIdentifier);
        }
        validate_title(&self.name)?;
        if let Some(u) = &self.units {
            validate_title(u)?;
        }
        if let ConstantValue::Number(v) = self.value
            && !v.is_finite()
        {
            return Err(DocError::InvalidCellValue { archetype: crate::TableKind::Column, row: 0, col: 0, reason: "info_constant.value" });
        }
        Ok(())
    }
}

impl<'de> serde::Deserialize<'de> for InfoConstant {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(serde::Deserialize)]
        struct RawInfoConstant {
            name: String,
            value: ConstantValue,
            units: Option<String>,
        }

        let raw = RawInfoConstant::deserialize(deserializer)?;
        let entry = Self { name: raw.name, value: raw.value, units: raw.units };
        entry.validate().map_err(serde::de::Error::custom)?;
        Ok(entry)
    }
}

/// Two-pane laboratory metadata and experimental notes sheet paired with (or shared across) one or
/// more [`crate::DataSheet`]s.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct InfoSheet {
    /// Project-unique sheet identifier.
    pub id: SheetId,
    /// Display title shown in the `Info` navigator folder.
    pub title: String,
    /// Upstream [`crate::DataSheet`] identifiers paired with this metadata sheet.
    pub linked_data_sheets: Vec<SheetId>,
    /// Ordered list of structured key–value experimental constants (left pane).
    pub constants: Vec<InfoConstant>,
    /// Free-form Markdown protocol and experiment notes (right pane).
    pub notes_markdown: String,
    /// Canvas sticky-note callouts attached to this info sheet.
    pub floating_notes: Vec<FloatingNote>,
    /// Optional navigator color swatch; `None` uses no color badge.
    pub color_tag: Option<Rgba8>,
}

impl InfoSheet {
    /// Create a new `InfoSheet` pre-seeded with [`STANDARD_INFO_KEYS`].
    pub fn new_preseeded(id: impl Into<SheetId>, title: impl Into<String>) -> Result<Self, DocError> {
        let id = id.into();
        id.validate_non_empty_bounded()?;
        let title = title.into();
        validate_title(&title)?;
        let constants =
            STANDARD_INFO_KEYS.iter().map(|&key| InfoConstant { name: key.to_owned(), value: ConstantValue::Empty, units: None }).collect();
        Ok(Self { id, title, linked_data_sheets: Vec::new(), constants, notes_markdown: String::new(), floating_notes: Vec::new(), color_tag: None })
    }

    /// Validate structural invariants (identifier bounds, title bounds, finite constants, and valid floating notes).
    pub fn validate(&self) -> Result<(), DocError> {
        self.id.validate_non_empty_bounded()?;
        validate_title(&self.title)?;
        for constant in &self.constants {
            constant.validate()?;
        }
        for note in &self.floating_notes {
            note.validate()?;
        }
        Ok(())
    }

    /// Look up a constant by case-insensitive trimmed name.
    #[must_use]
    pub fn get_constant(&self, name: &str) -> Option<&InfoConstant> {
        let needle = name.trim();
        self.constants.iter().find(|c| c.name.trim().eq_ignore_ascii_case(needle))
    }

    /// Upsert a named constant (`info.setConstant`), updating in place if the key already exists
    /// or appending a new row otherwise.
    ///
    /// When `units` is `None`, existing units on an updated constant are preserved (and new constants
    /// receive `None`). Passing `Some(None)` explicitly clears existing units, and `Some(Some(u))`
    /// sets `u`.
    pub fn upsert_constant(&mut self, name: impl Into<String>, value: ConstantValue, units: Option<Option<String>>) -> Result<(), DocError> {
        let entry = InfoConstant::new(name, value, units.clone().flatten())?;
        let needle = entry.name.trim();
        if let Some(existing) = self.constants.iter_mut().find(|c| c.name.trim().eq_ignore_ascii_case(needle)) {
            existing.value = entry.value;
            if let Some(new_units) = units {
                existing.units = new_units;
            }
        } else {
            self.constants.push(entry);
        }
        Ok(())
    }

    /// Upsert a named constant (`info.setConstant`), updating in place if the key already exists
    /// or appending a new row otherwise.
    pub fn set_constant(&mut self, name: impl Into<String>, value: ConstantValue, units: Option<String>) -> Result<(), DocError> {
        self.upsert_constant(name, value, Some(units))
    }

    /// Remove a constant by case-insensitive name, returning `true` if one was removed.
    pub fn remove_constant(&mut self, name: &str) -> bool {
        let needle = name.trim();
        let before = self.constants.len();
        self.constants.retain(|c| !c.name.trim().eq_ignore_ascii_case(needle));
        self.constants.len() < before
    }
}

impl<'de> serde::Deserialize<'de> for InfoSheet {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(serde::Deserialize)]
        struct RawInfoSheet {
            id: SheetId,
            title: String,
            linked_data_sheets: Vec<SheetId>,
            constants: Vec<InfoConstant>,
            notes_markdown: String,
            floating_notes: Vec<FloatingNote>,
            color_tag: Option<Rgba8>,
        }

        let raw = RawInfoSheet::deserialize(deserializer)?;
        let sheet = Self {
            id: raw.id,
            title: raw.title,
            linked_data_sheets: raw.linked_data_sheets,
            constants: raw.constants,
            notes_markdown: raw.notes_markdown,
            floating_notes: raw.floating_notes,
            color_tag: raw.color_tag,
        };
        sheet.validate().map_err(serde::de::Error::custom)?;
        Ok(sheet)
    }
}
