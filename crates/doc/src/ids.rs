//! Stable sheet/note identifiers, physical point geometry, collapsible `FloatingNote` callouts,
//! and `HookSource` constant-hooking descriptors (`docs/implementation/data-tables-and-transforms.md` §3).

use std::fmt;

use curvebook_palette::Rgba8;

use crate::error::{DocError, MAX_SHEET_ID_BYTES};

/// Stable project-unique identifier for a `DataSheet`, `InfoSheet`, `ResultSheet`, `GraphSheet`,
/// or `LayoutSheet`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct SheetId(String);

impl SheetId {
    /// Construct a `SheetId` from any string; callers parsing untrusted input should prefer [`Self::try_new`].
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Construct a validated non-empty, hook-safe `SheetId`.
    pub fn try_new(id: impl Into<String>) -> Result<Self, DocError> {
        let s = id.into();
        if s.trim().is_empty() {
            return Err(DocError::EmptyIdentifier);
        }
        if s.len() > MAX_SHEET_ID_BYTES {
            return Err(DocError::InvalidIdentifier { reason: "sheet_id must be at most 256 UTF-8 bytes" });
        }
        if s.chars().any(|ch| ch.is_whitespace() || ch.is_control()) {
            return Err(DocError::InvalidIdentifier { reason: "sheet_id must not contain whitespace or control characters" });
        }
        if s.contains(':') || s.contains("{{") || s.contains("}}") {
            return Err(DocError::InvalidIdentifier { reason: "sheet_id must not contain hook delimiter sequences (`:`, `{{`, `}}`)" });
        }
        Ok(Self(s))
    }

    /// Borrow the underlying identifier text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SheetId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for SheetId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for SheetId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

/// Stable identifier for a [`FloatingNote`] callout attached to a sheet.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct NoteId(String);

impl NoteId {
    /// Construct a `NoteId` from an arbitrary identifier string.
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Borrow the underlying note identifier string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for NoteId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// 2D physical size in typographic points (`1 pt = 1/72 in`).
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SizePt {
    /// Horizontal extent in typographic points (`>= 0.0` when constructed via [`Self::new`]).
    pub width: f64,
    /// Vertical extent in typographic points (`>= 0.0` when constructed via [`Self::new`]).
    pub height: f64,
}

impl SizePt {
    /// Construct a non-negative physical size in points, clamping negative or non-finite inputs to `0.0`.
    #[must_use]
    pub fn new(width: f64, height: f64) -> Self {
        Self { width: if width.is_finite() { width.max(0.0) } else { 0.0 }, height: if height.is_finite() { height.max(0.0) } else { 0.0 } }
    }
}

impl Default for SizePt {
    fn default() -> Self {
        Self { width: 360.0, height: 260.0 }
    }
}

/// Axis-aligned rectangle in typographic points (`1 pt = 1/72 in`).
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RectPt {
    /// Left origin coordinate in typographic points (`0.0` at canvas left).
    pub x: f64,
    /// Top origin coordinate in typographic points (`0.0` at canvas top).
    pub y: f64,
    /// Horizontal width in typographic points (`>= 0.0` when constructed via [`Self::new`]).
    pub width: f64,
    /// Vertical height in typographic points (`>= 0.0` when constructed via [`Self::new`]).
    pub height: f64,
}

impl RectPt {
    /// Construct a finite rectangle in points, mapping non-finite origins to `0.0` and clamping negative/non-finite dimensions to `0.0`.
    #[must_use]
    pub fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            x: if x.is_finite() { x } else { 0.0 },
            y: if y.is_finite() { y } else { 0.0 },
            width: if width.is_finite() { width.max(0.0) } else { 0.0 },
            height: if height.is_finite() { height.max(0.0) } else { 0.0 },
        }
    }
}

/// Collapsible rich-text sticky note attachable to any sheet in a project
/// (`docs/implementation/data-tables-and-transforms.md` §3).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FloatingNote {
    /// Stable identifier unique within the owning sheet's `floating_notes` list.
    pub id: NoteId,
    /// Background sRGB color of the sticky note card (defaults to [`Self::DEFAULT_FILL`]).
    pub fill_color: Rgba8,
    /// Canvas position and dimensions of the expanded callout in typographic points.
    pub bounds_pt: RectPt,
    /// When `true`, renders as a compact collapsed badge rather than an expanded note card.
    pub minimized: bool,
    /// Markdown body text (supports inline `{{info:...}}` and `{{result:...}}` constant hooks).
    pub body_markdown: String,
}

impl FloatingNote {
    /// Default warm pastel note fill with high black-text contrast.
    pub const DEFAULT_FILL: Rgba8 = Rgba8::rgb(0xFF, 0xF7, 0xCC);

    /// Construct an expanded `FloatingNote` with [`Self::DEFAULT_FILL`] background.
    #[must_use]
    pub fn new(id: impl Into<String>, bounds_pt: RectPt, body_markdown: impl Into<String>) -> Self {
        Self { id: NoteId::new(id), fill_color: Self::DEFAULT_FILL, bounds_pt, minimized: false, body_markdown: body_markdown.into() }
    }
}

/// Upstream source for a hooked constant used in regression parameter constraints, normalization
/// anchors, or dynamic figure/layout text spans (`{{info:...}}`, `{{result:...}}`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum HookSource {
    /// Named constant on an `InfoSheet` (`info_sheet_id: None` resolves the paired or primary info sheet).
    InfoConstant {
        /// Explicit upstream [`InfoSheet`](crate::InfoSheet) identifier, or `None` to resolve the paired info sheet of the active source table (falling back to the project's primary info sheet).
        info_sheet_id: Option<SheetId>,
        /// Case-insensitive [`InfoConstant::name`](crate::InfoConstant::name) key on the resolved info sheet.
        constant_name: String,
    },
    /// Named scalar output on an upstream `ResultSheet` (e.g., fitted `EC50`, `Kd`, `r2`, or control mean).
    ResultScalar {
        /// Identifier of the upstream [`ResultSheet`](crate::ResultSheet) publishing the scalar.
        result_sheet_id: SheetId,
        /// Case-insensitive [`ResultScalarEntry::key`](crate::ResultScalarEntry::key) name (for example `"LogEC50"`, `"Km"`, or `"p"`).
        parameter_key: String,
        /// Optional zero-based data-set column index when a fit or analysis publishes per-series scalars; `None` matches global scalars or the first matching entry.
        data_set_index: Option<usize>,
    },
    /// Numeric value parsed from a data-set column title (e.g., inhibitor concentration `[I]` in enzyme kinetics).
    ColumnTitleValue {
        /// Identifier of the upstream [`DataSheet`](crate::DataSheet) owning the column header.
        data_sheet_id: SheetId,
        /// Zero-based data-set column index whose title is parsed as a finite `f64`.
        data_set_index: usize,
    },
}

impl HookSource {
    /// Direct upstream `SheetId` referenced by this hook, when explicit.
    #[must_use]
    pub fn upstream_sheet_id(&self) -> Option<&SheetId> {
        match self {
            Self::InfoConstant { info_sheet_id, .. } => info_sheet_id.as_ref(),
            Self::ResultScalar { result_sheet_id, .. } => Some(result_sheet_id),
            Self::ColumnTitleValue { data_sheet_id, .. } => Some(data_sheet_id),
        }
    }
}

/// Extract inline template hooks (`{{info:Name}}`, `{{info:sheet-id:Name}}`,
/// `{{result:fit-1:EC50}}`, `{{result:fit-1:0:EC50}}`) from rich text or titles without panicking.
#[must_use]
pub fn extract_template_hooks(text: &str) -> Vec<HookSource> {
    let mut hooks = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find("{{") {
        let after_open = &rest[open + 2..];
        let Some(close) = after_open.find("}}") else {
            break;
        };
        let token = after_open[..close].trim();
        if let Some(hook) = parse_single_hook_token(token) {
            hooks.push(hook);
        }
        rest = &after_open[close + 2..];
    }
    hooks
}

fn parse_single_hook_token(token: &str) -> Option<HookSource> {
    let parts: Vec<&str> = token.split(':').map(str::trim).collect();
    match parts.as_slice() {
        ["info", name] if !name.is_empty() => Some(HookSource::InfoConstant { info_sheet_id: None, constant_name: (*name).to_owned() }),
        ["info", sheet, name] if !sheet.is_empty() && !name.is_empty() => {
            Some(HookSource::InfoConstant { info_sheet_id: Some(SheetId::new(*sheet)), constant_name: (*name).to_owned() })
        }
        ["result", sheet, param] if !sheet.is_empty() && !param.is_empty() => {
            Some(HookSource::ResultScalar { result_sheet_id: SheetId::new(*sheet), parameter_key: (*param).to_owned(), data_set_index: None })
        }
        ["result", sheet, ds_str, param] if !sheet.is_empty() && !param.is_empty() => {
            let ds = ds_str.parse::<usize>().ok()?;
            Some(HookSource::ResultScalar { result_sheet_id: SheetId::new(*sheet), parameter_key: (*param).to_owned(), data_set_index: Some(ds) })
        }
        _ => None,
    }
}
