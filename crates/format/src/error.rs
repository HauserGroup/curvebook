//! Structured errors and non-fatal import diagnostics for `lumencraft-format`.

use std::fmt;

use lumencraft_doc::{DocError, ProjectDocument, SheetId};

/// Structured error type returned by all archive, CSV/TSV, XML, and interchange operations.
#[derive(Debug, Clone, PartialEq)]
pub enum FormatError {
    Io(String),
    Json(String),
    Doc(DocError),
    InvalidZip { reason: String },
    CorruptCrc { entry: String, expected: u32, actual: u32 },
    UnsupportedCompression { entry: String, method: u16 },
    ZipBomb { reason: String },
    UnsafePath { path: String },
    DuplicateEntry { path: String },
    MissingArchiveEntry { path: String },
    UnsupportedSchemaVersion { version: u32, max_supported: u32 },
    InvalidArchiveKind { expected: &'static str, found: &'static str },
    DependencyCycle { cycle: Vec<SheetId> },
    NonFiniteFloat { location: String },
    CsvParse { line: usize, reason: String },
    XmlParse { offset: usize, reason: String },
    XmlDepthExceeded { depth: usize, max: usize },
    UnsupportedFormat { reason: String },
    InvalidPipeline { reason: String },
}

impl fmt::Display for FormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(msg) => write!(f, "I/O error: {msg}"),
            Self::Json(msg) => write!(f, "JSON serialization error: {msg}"),
            Self::Doc(err) => write!(f, "document validation error: {err}"),
            Self::InvalidZip { reason } => write!(f, "invalid ZIP archive: {reason}"),
            Self::CorruptCrc { entry, expected, actual } => {
                write!(f, "CRC-32 mismatch for archive entry `{entry}`: expected {expected:#010x}, got {actual:#010x}")
            }
            Self::UnsupportedCompression { entry, method } => {
                write!(f, "unsupported ZIP compression method {method} for entry `{entry}`")
            }
            Self::ZipBomb { reason } => write!(f, "archive security limit exceeded: {reason}"),
            Self::UnsafePath { path } => write!(f, "unsafe archive entry path `{path}`"),
            Self::DuplicateEntry { path } => write!(f, "duplicate archive entry `{path}`"),
            Self::MissingArchiveEntry { path } => write!(f, "missing required archive entry `{path}`"),
            Self::UnsupportedSchemaVersion { version, max_supported } => {
                write!(f, "unsupported schema version {version} (max supported: {max_supported})")
            }
            Self::InvalidArchiveKind { expected, found } => {
                write!(f, "unexpected archive kind: expected {expected}, found {found}")
            }
            Self::DependencyCycle { cycle } => {
                let path: Vec<&str> = cycle.iter().map(SheetId::as_str).collect();
                write!(f, "circular sheet dependency detected: {}", path.join(" -> "))
            }
            Self::NonFiniteFloat { location } => write!(f, "non-finite floating-point value at {location}"),
            Self::CsvParse { line, reason } => write!(f, "CSV parse error at line {line}: {reason}"),
            Self::XmlParse { offset, reason } => write!(f, "XML parse error at byte offset {offset}: {reason}"),
            Self::XmlDepthExceeded { depth, max } => write!(f, "XML nesting depth {depth} exceeds maximum {max}"),
            Self::UnsupportedFormat { reason } => write!(f, "unsupported format: {reason}"),
            Self::InvalidPipeline { reason } => write!(f, "invalid import pipeline: {reason}"),
        }
    }
}

impl std::error::Error for FormatError {}

impl From<DocError> for FormatError {
    fn from(value: DocError) -> Self {
        Self::Doc(value)
    }
}

impl From<std::io::Error> for FormatError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value.to_string())
    }
}

impl From<serde_json::Error> for FormatError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value.to_string())
    }
}

/// Classification of a non-fatal compatibility warning emitted during external import.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum ImportWarningKind {
    UnsupportedTableType,
    UnsupportedChunk,
    IgnoredSection,
    TruncatedTitle,
    CoercedValue,
    MissingMetadata,
}

/// Non-fatal compatibility note emitted during `.pzfx` or `.prism` project import.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ImportWarning {
    pub kind: ImportWarningKind,
    pub source: String,
    pub message: String,
}

impl ImportWarning {
    #[must_use]
    pub fn new(kind: ImportWarningKind, source: impl Into<String>, message: impl Into<String>) -> Self {
        Self { kind, source: source.into(), message: message.into() }
    }
}

/// Result of importing an external project container (`.pzfx` or `.prism`).
#[derive(Debug, Clone, PartialEq)]
pub struct ImportReport {
    pub document: ProjectDocument,
    pub warnings: Vec<ImportWarning>,
}
