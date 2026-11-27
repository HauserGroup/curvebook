//! Bounded XML 1.0 DOM parser and deterministic writer backed by `quick-xml` for clean-room
//! scientific XML interchange.
//!
//! Capabilities:
//! - Automatic encoding detection (`UTF-8` with/without BOM, `UTF-16LE`/`UTF-16BE` with/without
//!   BOM, and declared `ISO-8859-1`, `Windows-1252`, `MacRoman` via Unicode Consortium lookup
//!   tables; unknown encodings fail with [`FormatError::UnsupportedFormat`]).
//! - Optional stop-after-root parsing that reports unparsed `trailing_bytes` without decoding the
//!   tail.
//! - Configurable opaque-element body skipping with exact skipped byte counts while still charging
//!   against raw input limits (enabling caller-configured caps up to 128 MiB when opaque skipping
//!   is active).
//! - Whitespace-only text nodes preserved in mixed content without counting against `max_nodes`
//!   (while raw bytes still count against `max_bytes`).
//! - Predefined XML 1.0 entities, full W3C HTML 4.01 named character entities, scientific aliases,
//!   and numeric character references, with `<!DOCTYPE` and `<!ENTITY` strictly forbidden.
//! - Deterministic [`XmlWriter`] preserving caller-controlled attribute order, escaping
//!   `& < > " '`, and rejecting characters outside the XML 1.0 `Char` production.

mod encoding;
mod entities;
mod writer;

use std::collections::HashSet;

use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};

pub use encoding::{XmlEncoding, detect_xml_encoding};
pub use entities::{decode_xml_entities, is_valid_xml10_char, lookup_named_xml_entity};
pub use writer::XmlWriter;

use encoding::{SingleByteToUtf8Reader, Utf16ToUtf8Reader, tail_is_only_xml_whitespace, validate_str_declared_encoding};
use entities::decode_xml_entity_name;

use crate::error::FormatError;

/// Default maximum XML payload size in bytes (64 MiB).
pub const MAX_XML_BYTES: usize = 64 * 1024 * 1024;

/// Maximum raw XML payload size in bytes permitted when opaque-body skipping is enabled (128 MiB).
pub const MAX_XML_OPAQUE_BYTES: usize = 128 * 1024 * 1024;

/// Default maximum XML element nesting depth.
pub const MAX_XML_DEPTH: usize = 128;

/// Default maximum number of non-whitespace XML nodes (elements, attributes, text, CDATA) in a
/// single document.
pub const MAX_XML_NODES: usize = 2_000_000;

/// Security limits for XML parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct XmlLimits {
    pub max_bytes: usize,
    pub max_depth: usize,
    pub max_nodes: usize,
}

impl Default for XmlLimits {
    fn default() -> Self {
        Self { max_bytes: MAX_XML_BYTES, max_depth: MAX_XML_DEPTH, max_nodes: MAX_XML_NODES }
    }
}

/// Configuration for [`parse_xml_bytes_with_options`] and [`parse_xml_with_options`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct XmlParseOptions {
    pub limits: XmlLimits,
    /// When `true`, stop parsing immediately after the root element closes and report any
    /// remaining non-whitespace bytes in [`ParsedXml::trailing_bytes`] without decoding them.
    /// When `false` (default), trailing content or multiple root elements remain errors.
    pub stop_after_root: bool,
    /// Local element names whose bodies are skipped without constructing child DOM nodes.
    /// Skipped byte counts are recorded in [`ParsedXml::skipped_opaque`] while still counting
    /// against `limits.max_bytes`.
    pub opaque_elements: Vec<String>,
}

impl XmlParseOptions {
    /// Replace security limits.
    #[must_use]
    pub const fn with_limits(mut self, limits: XmlLimits) -> Self {
        self.limits = limits;
        self
    }

    /// Configure whether parsing stops immediately once the root element closes.
    #[must_use]
    pub const fn with_stop_after_root(mut self, stop_after_root: bool) -> Self {
        self.stop_after_root = stop_after_root;
        self
    }

    /// Configure local element names whose bodies should be skipped as opaque byte payloads.
    #[must_use]
    pub fn with_opaque_elements<I, S>(mut self, names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.opaque_elements = names.into_iter().map(Into::into).filter(|name| !name.is_empty()).collect();
        self
    }

    /// Validate that the configured byte cap respects the 64 MiB default ceiling (or up to
    /// 128 MiB when at least one opaque element name is configured).
    pub fn validate(&self) -> Result<(), FormatError> {
        let has_opaque = self.opaque_elements.iter().any(|name| !name.is_empty());
        let ceiling = if has_opaque { MAX_XML_OPAQUE_BYTES } else { MAX_XML_BYTES };
        if self.limits.max_bytes > ceiling {
            return Err(FormatError::XmlParse {
                offset: 0,
                reason: format!(
                    "configured XML byte cap {} exceeds ceiling {} (opaque-body skipping {})",
                    self.limits.max_bytes,
                    ceiling,
                    if has_opaque { "enabled" } else { "disabled" }
                ),
            });
        }
        Ok(())
    }
}

/// Summary of an opaque element whose body was skipped without constructing child DOM nodes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedOpaqueElement {
    /// Full tag name as written in the XML document (including any namespace prefix).
    pub name: String,
    /// Raw byte length of the skipped body between the start tag `>` and matching end tag `<`.
    pub body_bytes: usize,
}

impl SkippedOpaqueElement {
    /// Local element name stripped of any namespace prefix (`ns:Tag` -> `Tag`).
    #[must_use]
    pub fn local_name(&self) -> &str {
        local_name_of(&self.name)
    }
}

/// Result of parsing an XML document with encoding detection, optional root termination, and
/// optional opaque-body skipping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedXml {
    pub root: XmlElement,
    pub encoding: XmlEncoding,
    /// Number of unparsed raw bytes remaining after the root element closed when non-whitespace
    /// trailing data is present; `0` when the root is followed only by EOF or XML whitespace.
    pub trailing_bytes: usize,
    /// Opaque elements whose bodies were skipped during parsing, in document order.
    pub skipped_opaque: Vec<SkippedOpaqueElement>,
}

impl ParsedXml {
    /// Cumulative raw bytes skipped across all recorded opaque element bodies.
    #[must_use]
    pub fn total_opaque_bytes(&self) -> usize {
        self.skipped_opaque.iter().map(|entry| entry.body_bytes).fold(0_usize, usize::saturating_add)
    }
}

/// Child item inside an [`XmlElement`], preserving exact mixed-content order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum XmlNode {
    Text(String),
    Element(XmlElement),
}

/// Parsed XML element node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XmlElement {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<XmlNode>,
}

impl XmlElement {
    /// Local element name stripped of any namespace prefix (`ns:Tag` -> `Tag`).
    #[must_use]
    pub fn local_name(&self) -> &str {
        local_name_of(&self.name)
    }

    /// Look up an attribute value by exact or local name.
    #[must_use]
    pub fn attr(&self, key: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == key || k.rsplit_once(':').is_some_and(|(_, local)| local == key)).map(|(_, v)| v.as_str())
    }

    /// Iterate over direct child elements.
    pub fn child_elements(&self) -> impl Iterator<Item = &Self> {
        self.children.iter().filter_map(|n| match n {
            XmlNode::Element(el) => Some(el),
            XmlNode::Text(_) => None,
        })
    }

    /// Find first direct child element matching `local_name`.
    #[must_use]
    pub fn first_child(&self, local: &str) -> Option<&Self> {
        self.child_elements().find(|el| el.local_name() == local)
    }

    /// Iterate over all direct child elements matching `local_name`.
    pub fn children_named<'a>(&'a self, local: &'a str) -> impl Iterator<Item = &'a Self> {
        self.child_elements().filter(move |el| el.local_name() == local)
    }

    /// Collect concatenated plain text recursively across mixed content (e.g. `<Title>` containing
    /// `<Font>` or `<sup>`/`<sub>` formatting tags).
    #[must_use]
    pub fn plain_text(&self) -> String {
        let mut out = String::new();
        collect_plain_text(self, &mut out);
        out
    }
}

fn local_name_of(name: &str) -> &str {
    name.rsplit_once(':').map_or(name, |(_, local)| local)
}

fn collect_plain_text(el: &XmlElement, out: &mut String) {
    for node in &el.children {
        match node {
            XmlNode::Text(s) => out.push_str(s),
            XmlNode::Element(child) => collect_plain_text(child, out),
        }
    }
}

/// Parse an XML string into its root [`XmlElement`] using [`XmlLimits::default`].
pub fn parse_xml(input: &str) -> Result<XmlElement, FormatError> {
    parse_xml_with_limits(input, XmlLimits::default())
}

/// Parse an XML string enforcing explicit [`XmlLimits`].
pub fn parse_xml_with_limits(input: &str, limits: XmlLimits) -> Result<XmlElement, FormatError> {
    let options = XmlParseOptions { limits, stop_after_root: false, opaque_elements: Vec::new() };
    parse_str_inner(input, &options, false).map(|parsed| parsed.root)
}

/// Parse an already-decoded UTF-8 XML string with full [`XmlParseOptions`].
pub fn parse_xml_with_options(input: &str, options: &XmlParseOptions) -> Result<ParsedXml, FormatError> {
    parse_str_inner(input, options, true)
}

/// Parse raw XML bytes with automatic encoding detection and strict [`XmlParseOptions::default`].
pub fn parse_xml_bytes(bytes: &[u8]) -> Result<ParsedXml, FormatError> {
    parse_xml_bytes_with_options(bytes, &XmlParseOptions::default())
}

/// Parse raw XML bytes with automatic encoding detection and explicit [`XmlParseOptions`].
pub fn parse_xml_bytes_with_options(bytes: &[u8], options: &XmlParseOptions) -> Result<ParsedXml, FormatError> {
    options.validate()?;
    check_raw_byte_cap(bytes.len(), options.limits.max_bytes)?;

    let (encoding, bom_len) = detect_xml_encoding(bytes)?;
    let content = bytes.get(bom_len..).unwrap_or_default();
    let mut builder = XmlTreeBuilder::new(options);

    let consumed_end = match encoding {
        XmlEncoding::Utf8 => builder.parse_utf8_slice(content, bom_len)?,
        XmlEncoding::Utf16Le => builder.parse_utf16_slice(content, bom_len, false)?,
        XmlEncoding::Utf16Be => builder.parse_utf16_slice(content, bom_len, true)?,
        XmlEncoding::Iso8859_1 | XmlEncoding::Windows1252 | XmlEncoding::MacRoman => builder.parse_single_byte_slice(content, bom_len, encoding)?,
    };

    let trailing_bytes = compute_trailing_bytes(content, consumed_end, encoding, options.stop_after_root);
    let root = builder.finish_root(bom_len.saturating_add(consumed_end))?;
    Ok(ParsedXml { root, encoding, trailing_bytes, skipped_opaque: builder.skipped_opaque })
}

fn parse_str_inner(input: &str, options: &XmlParseOptions, validate_ceiling: bool) -> Result<ParsedXml, FormatError> {
    if validate_ceiling {
        options.validate()?;
    }
    check_raw_byte_cap(input.len(), options.limits.max_bytes)?;

    let (src, bom_len) = match input.strip_prefix('\u{FEFF}') {
        Some(stripped) => (stripped, 3_usize),
        None => (input, 0_usize),
    };
    validate_str_declared_encoding(src)?;

    let mut builder = XmlTreeBuilder::new(options);
    let consumed_end = builder.parse_utf8_slice(src.as_bytes(), bom_len)?;
    let trailing_bytes = compute_trailing_bytes(src.as_bytes(), consumed_end, XmlEncoding::Utf8, options.stop_after_root);
    let root = builder.finish_root(bom_len.saturating_add(consumed_end))?;
    Ok(ParsedXml { root, encoding: XmlEncoding::Utf8, trailing_bytes, skipped_opaque: builder.skipped_opaque })
}

fn check_raw_byte_cap(len: usize, max_bytes: usize) -> Result<(), FormatError> {
    if len > max_bytes {
        return Err(FormatError::XmlParse { offset: max_bytes, reason: format!("XML byte length {len} exceeds limit {max_bytes}") });
    }
    Ok(())
}

fn compute_trailing_bytes(content: &[u8], consumed_end: usize, encoding: XmlEncoding, stop_after_root: bool) -> usize {
    if !stop_after_root {
        return 0;
    }
    let tail = content.get(consumed_end..).unwrap_or_default();
    if tail.is_empty() || tail_is_only_xml_whitespace(tail, encoding) { 0 } else { tail.len() }
}

struct ActiveOpaque {
    element: XmlElement,
    body_start: usize,
    depth: usize,
}

struct XmlTreeBuilder<'opts> {
    limits: XmlLimits,
    stop_after_root: bool,
    opaque_elements: &'opts [String],
    dom_bytes_cap: usize,
    dom_bytes_used: usize,
    node_count: usize,
    stack: Vec<XmlElement>,
    pending_text: String,
    root: Option<XmlElement>,
    active_opaque: Option<ActiveOpaque>,
    skipped_opaque: Vec<SkippedOpaqueElement>,
}

enum EventStep {
    Continue,
    StopAtRoot,
    Eof,
}

impl<'opts> XmlTreeBuilder<'opts> {
    fn new(options: &'opts XmlParseOptions) -> Self {
        Self {
            limits: options.limits,
            stop_after_root: options.stop_after_root,
            opaque_elements: &options.opaque_elements,
            dom_bytes_cap: options.limits.max_bytes.min(MAX_XML_BYTES),
            dom_bytes_used: 0,
            node_count: 0,
            stack: Vec::new(),
            pending_text: String::new(),
            root: None,
            active_opaque: None,
            skipped_opaque: Vec::new(),
        }
    }

    fn is_opaque_tag(&self, name: &str) -> bool {
        if self.opaque_elements.is_empty() {
            return false;
        }
        let local = local_name_of(name);
        self.opaque_elements.iter().any(|configured| configured == local)
    }

    fn charge_dom_bytes(&mut self, added: usize, offset: usize) -> Result<(), FormatError> {
        let next = self.dom_bytes_used.saturating_add(added);
        if next > self.dom_bytes_cap {
            return Err(FormatError::XmlParse { offset, reason: format!("parsed XML DOM byte length {next} exceeds limit {}", self.dom_bytes_cap) });
        }
        self.dom_bytes_used = next;
        Ok(())
    }

    fn bump_node_count(&mut self, offset: usize, kind: &str) -> Result<(), FormatError> {
        self.node_count += 1;
        if self.node_count > self.limits.max_nodes {
            return Err(FormatError::XmlParse { offset, reason: format!("XML {kind} count exceeds node limit {}", self.limits.max_nodes) });
        }
        Ok(())
    }

    fn flush_pending_text(&mut self, offset: usize) -> Result<(), FormatError> {
        if self.pending_text.is_empty() {
            return Ok(());
        }
        let text = std::mem::take(&mut self.pending_text);
        if !self.stack.is_empty() {
            if !is_xml_whitespace_only(&text) {
                self.bump_node_count(offset, "text node")?;
            }
            self.charge_dom_bytes(text.len(), offset)?;
            if let Some(parent) = self.stack.last_mut() {
                parent.children.push(XmlNode::Text(text));
            }
        }
        Ok(())
    }

    fn build_element(&mut self, start: &BytesStart<'_>, offset: usize) -> Result<XmlElement, FormatError> {
        let depth = self.stack.len() + 1;
        if depth > self.limits.max_depth {
            return Err(FormatError::XmlDepthExceeded { depth, max: self.limits.max_depth });
        }
        if self.stack.is_empty() && self.root.is_some() {
            return Err(FormatError::XmlParse { offset, reason: String::from("multiple root XML elements") });
        }
        self.bump_node_count(offset, "element")?;

        let name_qname = start.name();
        let name: &str = name_qname.as_ref();
        if name.is_empty() {
            return Err(FormatError::XmlParse { offset, reason: String::from("empty XML tag name") });
        }
        self.charge_dom_bytes(name.len(), offset)?;

        let attrs = self.read_attributes(start, offset)?;
        Ok(XmlElement { name: name.to_owned(), attrs, children: Vec::new() })
    }

    fn read_attributes(&mut self, start: &BytesStart<'_>, offset: usize) -> Result<Vec<(String, String)>, FormatError> {
        let mut attrs: Vec<(String, String)> = Vec::new();
        let mut seen: Option<HashSet<String>> = None;

        for attr_res in start.attributes().with_checks(false) {
            let attr = attr_res.map_err(|err| FormatError::XmlParse { offset, reason: format!("malformed XML attribute: {err}") })?;
            self.bump_node_count(offset, "attribute")?;
            let key: &str = attr.key.as_ref();
            if key.is_empty() {
                return Err(FormatError::XmlParse { offset, reason: String::from("expected attribute name") });
            }
            if is_duplicate_attr(&attrs, &mut seen, key) {
                return Err(FormatError::XmlParse { offset, reason: format!("duplicate attribute `{key}`") });
            }
            let raw_val: &str = attr.value.as_ref();
            let decoded = decode_xml_entities(raw_val, offset)?;
            validate_decoded_xml10_chars(&decoded, offset)?;
            self.charge_dom_bytes(key.len().saturating_add(decoded.len()), offset)?;
            attrs.push((key.to_owned(), decoded));
        }

        Ok(attrs)
    }

    fn attach_completed_element(&mut self, element: XmlElement) -> EventStep {
        if let Some(parent) = self.stack.last_mut() {
            parent.children.push(XmlNode::Element(element));
            EventStep::Continue
        } else {
            self.root = Some(element);
            if self.stop_after_root { EventStep::StopAtRoot } else { EventStep::Continue }
        }
    }

    fn handle_text(&mut self, raw: &str, offset: usize) -> Result<(), FormatError> {
        if self.stack.is_empty() {
            if !is_xml_whitespace_only(raw) {
                return Err(FormatError::XmlParse { offset, reason: String::from("unexpected character data outside root element") });
            }
            return Ok(());
        }
        let decoded = decode_xml_entities(raw, offset)?;
        validate_decoded_xml10_chars(&decoded, offset)?;
        self.pending_text.push_str(&decoded);
        Ok(())
    }

    fn handle_general_ref(&mut self, entity: &str, offset: usize) -> Result<(), FormatError> {
        if self.stack.is_empty() {
            return Err(FormatError::XmlParse { offset, reason: String::from("unexpected character data outside root element") });
        }
        let ch = decode_xml_entity_name(entity, offset)?;
        self.pending_text.push(ch);
        Ok(())
    }

    fn handle_cdata(&mut self, cdata: &str, offset: usize) -> Result<(), FormatError> {
        if self.stack.is_empty() {
            return Err(FormatError::XmlParse { offset, reason: String::from("unexpected CDATA outside root element") });
        }
        validate_decoded_xml10_chars(cdata, offset)?;
        self.flush_pending_text(offset)?;
        if !cdata.is_empty() {
            self.bump_node_count(offset, "CDATA node")?;
            self.charge_dom_bytes(cdata.len(), offset)?;
            if let Some(parent) = self.stack.last_mut() {
                parent.children.push(XmlNode::Text(cdata.to_owned()));
            }
        }
        Ok(())
    }

    fn handle_opaque_event(&mut self, event: Event<'_>, tag_start_offset: usize, offset: usize) -> Result<EventStep, FormatError> {
        match event {
            Event::Start(_) => {
                if let Some(active) = self.active_opaque.as_mut() {
                    active.depth += 1;
                    let total_depth = self.stack.len().saturating_add(active.depth);
                    if total_depth > self.limits.max_depth {
                        return Err(FormatError::XmlDepthExceeded { depth: total_depth, max: self.limits.max_depth });
                    }
                }
                Ok(EventStep::Continue)
            }
            Event::Empty(_) => {
                if let Some(active) = self.active_opaque.as_ref() {
                    let total_depth = self.stack.len().saturating_add(active.depth).saturating_add(1);
                    if total_depth > self.limits.max_depth {
                        return Err(FormatError::XmlDepthExceeded { depth: total_depth, max: self.limits.max_depth });
                    }
                }
                Ok(EventStep::Continue)
            }
            Event::End(end) => self.handle_opaque_end(end.name().as_ref(), tag_start_offset, offset),
            Event::DocType(_) => {
                Err(FormatError::XmlParse { offset, reason: String::from("DOCTYPE and external ENTITY declarations are forbidden") })
            }
            Event::Eof => Ok(EventStep::Eof),
            Event::Text(_) | Event::GeneralRef(_) | Event::CData(_) | Event::Comment(_) | Event::PI(_) | Event::Decl(_) => Ok(EventStep::Continue),
        }
    }

    fn handle_opaque_end(&mut self, close_name: &str, tag_start_offset: usize, offset: usize) -> Result<EventStep, FormatError> {
        let finish_now = match self.active_opaque.as_mut() {
            Some(active) if active.depth > 1 => {
                active.depth -= 1;
                false
            }
            Some(_) => true,
            None => false,
        };
        if !finish_now {
            return Ok(EventStep::Continue);
        }
        let Some(active) = self.active_opaque.take() else {
            return Ok(EventStep::Continue);
        };
        if active.element.name != close_name {
            return Err(FormatError::XmlParse {
                offset,
                reason: format!("mismatched closing tag `</{close_name}>`, expected `</{}>`", active.element.name),
            });
        }
        let body_bytes = tag_start_offset.saturating_sub(active.body_start);
        self.skipped_opaque.push(SkippedOpaqueElement { name: active.element.name.clone(), body_bytes });
        Ok(self.attach_completed_element(active.element))
    }

    fn handle_event(
        &mut self,
        event: Event<'_>,
        tag_start_offset: usize,
        after_event_offset: usize,
        diag_offset: usize,
    ) -> Result<EventStep, FormatError> {
        if self.active_opaque.is_some() {
            return self.handle_opaque_event(event, tag_start_offset, diag_offset);
        }
        match event {
            Event::Start(start) => {
                self.flush_pending_text(diag_offset)?;
                let el = self.build_element(&start, diag_offset)?;
                if self.is_opaque_tag(&el.name) {
                    self.active_opaque = Some(ActiveOpaque { element: el, body_start: after_event_offset, depth: 1 });
                } else {
                    self.stack.push(el);
                }
                Ok(EventStep::Continue)
            }
            Event::Empty(start) => {
                self.flush_pending_text(diag_offset)?;
                let el = self.build_element(&start, diag_offset)?;
                if self.is_opaque_tag(&el.name) {
                    self.skipped_opaque.push(SkippedOpaqueElement { name: el.name.clone(), body_bytes: 0 });
                }
                Ok(self.attach_completed_element(el))
            }
            Event::End(end) => {
                self.flush_pending_text(diag_offset)?;
                let close_qname = end.name();
                let close_name: &str = close_qname.as_ref();
                let Some(el) = self.stack.pop() else {
                    return Err(FormatError::XmlParse { offset: diag_offset, reason: format!("unexpected closing tag `</{close_name}>`") });
                };
                if el.name != close_name {
                    return Err(FormatError::XmlParse {
                        offset: diag_offset,
                        reason: format!("mismatched closing tag `</{close_name}>`, expected `</{}>`", el.name),
                    });
                }
                Ok(self.attach_completed_element(el))
            }
            Event::Text(text) => {
                self.handle_text(text.as_ref(), diag_offset)?;
                Ok(EventStep::Continue)
            }
            Event::GeneralRef(entity) => {
                self.handle_general_ref(entity.as_ref(), diag_offset)?;
                Ok(EventStep::Continue)
            }
            Event::CData(cdata) => {
                self.handle_cdata(cdata.as_ref(), diag_offset)?;
                Ok(EventStep::Continue)
            }
            Event::Comment(_) | Event::PI(_) | Event::Decl(_) => Ok(EventStep::Continue),
            Event::DocType(_) => {
                Err(FormatError::XmlParse { offset: diag_offset, reason: String::from("DOCTYPE and external ENTITY declarations are forbidden") })
            }
            Event::Eof => Ok(EventStep::Eof),
        }
    }

    fn parse_utf8_slice(&mut self, src: &[u8], bom_len: usize) -> Result<usize, FormatError> {
        let mut reader = Reader::from_reader(src);
        reader.config_mut().check_end_names = true;
        reader.config_mut().trim_text(false);

        loop {
            let before = usize::try_from(reader.buffer_position()).unwrap_or(usize::MAX);
            let diag_offset = bom_len.saturating_add(before);
            let event = reader.read_event().map_err(|err| FormatError::XmlParse { offset: diag_offset, reason: format!("malformed XML: {err}") })?;
            let after = usize::try_from(reader.buffer_position()).unwrap_or(usize::MAX);
            match self.handle_event(event, before, after, diag_offset)? {
                EventStep::Continue => {}
                EventStep::StopAtRoot | EventStep::Eof => return Ok(after),
            }
        }
    }

    fn parse_utf16_slice(&mut self, src: &[u8], bom_len: usize, big_endian: bool) -> Result<usize, FormatError> {
        let adapter = Utf16ToUtf8Reader::new(src, big_endian);
        let mut reader = Reader::from_reader(adapter);
        reader.config_mut().check_end_names = true;
        reader.config_mut().trim_text(false);
        let mut scratch = Vec::new();

        loop {
            scratch.clear();
            let pre_pos = reader.get_ref().raw_input_pos();
            let diag_offset = bom_len.saturating_add(pre_pos);
            let event = match reader.read_event_into(&mut scratch) {
                Ok(ev) => ev,
                Err(err) => {
                    if let Some(mut decode_err) = reader.get_mut().take_error() {
                        if let FormatError::XmlParse { ref mut offset, .. } = decode_err {
                            *offset = offset.saturating_add(bom_len);
                        }
                        return Err(decode_err);
                    }
                    return Err(FormatError::XmlParse { offset: diag_offset, reason: format!("malformed XML: {err}") });
                }
            };
            let tag_start = reader.get_ref().last_chunk_start_input_pos();
            let after = reader.get_ref().raw_input_pos();
            match self.handle_event(event, tag_start, after, diag_offset)? {
                EventStep::Continue => {}
                EventStep::StopAtRoot | EventStep::Eof => return Ok(after),
            }
        }
    }

    fn parse_single_byte_slice(&mut self, src: &[u8], bom_len: usize, encoding: XmlEncoding) -> Result<usize, FormatError> {
        let adapter = SingleByteToUtf8Reader::new(src, encoding);
        let mut reader = Reader::from_reader(adapter);
        reader.config_mut().check_end_names = true;
        reader.config_mut().trim_text(false);
        let mut scratch = Vec::new();

        loop {
            scratch.clear();
            let pre_pos = reader.get_ref().raw_input_pos();
            let diag_offset = bom_len.saturating_add(pre_pos);
            let event = match reader.read_event_into(&mut scratch) {
                Ok(ev) => ev,
                Err(err) => {
                    if let Some(mut decode_err) = reader.get_mut().take_error() {
                        if let FormatError::XmlParse { ref mut offset, .. } = decode_err {
                            *offset = offset.saturating_add(bom_len);
                        }
                        return Err(decode_err);
                    }
                    return Err(FormatError::XmlParse { offset: diag_offset, reason: format!("malformed XML: {err}") });
                }
            };
            let tag_start = reader.get_ref().last_chunk_start_input_pos();
            let after = reader.get_ref().raw_input_pos();
            match self.handle_event(event, tag_start, after, diag_offset)? {
                EventStep::Continue => {}
                EventStep::StopAtRoot | EventStep::Eof => return Ok(after),
            }
        }
    }

    fn finish_root(&mut self, eof_offset: usize) -> Result<XmlElement, FormatError> {
        if let Some(opaque) = self.active_opaque.take() {
            return Err(FormatError::XmlParse { offset: eof_offset, reason: format!("unclosed opaque XML tag `<{}>`", opaque.element.name) });
        }
        if let Some(unclosed) = self.stack.last() {
            return Err(FormatError::XmlParse { offset: eof_offset, reason: format!("unclosed XML tag `<{}>`", unclosed.name) });
        }
        self.root.take().ok_or_else(|| FormatError::XmlParse { offset: eof_offset, reason: String::from("empty XML document") })
    }
}

fn is_xml_whitespace_only(text: &str) -> bool {
    text.bytes().all(|b| matches!(b, b' ' | b'\t' | b'\r' | b'\n'))
}

fn validate_decoded_xml10_chars(text: &str, offset: usize) -> Result<(), FormatError> {
    for ch in text.chars() {
        if !is_valid_xml10_char(ch) {
            return Err(FormatError::XmlParse { offset, reason: format!("forbidden XML 1.0 character U+{:04X}", ch as u32) });
        }
    }
    Ok(())
}

fn is_duplicate_attr(attrs: &[(String, String)], seen: &mut Option<HashSet<String>>, key: &str) -> bool {
    if let Some(set) = seen.as_mut() {
        return !set.insert(key.to_owned());
    }
    if attrs.iter().any(|(existing, _)| existing == key) {
        return true;
    }
    if attrs.len() >= 16 {
        let mut set: HashSet<String> = attrs.iter().map(|(k, _)| k.clone()).collect();
        set.insert(key.to_owned());
        *seen = Some(set);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::{XmlLimits, XmlNode, parse_xml, parse_xml_with_limits};
    use crate::error::FormatError;

    #[test]
    fn quick_xml_adapter_preserves_mixed_content_and_scientific_entities() {
        let xml = "\u{FEFF}<?xml version=\"1.0\"?>\n<!-- c -->\n<ns:Root a=\"1 &amp; &mu;M\" b='x'>pre &alpha;<b>mid</b>&pm;10&deg;<![CDATA[<raw>&amp;]]></ns:Root>";
        let root = parse_xml(xml).unwrap();
        assert_eq!(root.local_name(), "Root");
        assert_eq!(root.attr("a"), Some("1 & μM"));
        assert_eq!(root.attr("b"), Some("x"));
        assert_eq!(root.plain_text(), "pre αmid±10°<raw>&amp;");
        assert_eq!(root.children.len(), 4);
        assert_eq!(root.children[0], XmlNode::Text(String::from("pre α")));
        assert_eq!(root.children[2], XmlNode::Text(String::from("±10°")));
        assert_eq!(root.children[3], XmlNode::Text(String::from("<raw>&amp;")));
    }

    #[test]
    fn quick_xml_adapter_decodes_extended_scientific_and_xhtml_entities() {
        let xml = "<Root>&copy;&reg;&trade;&lsquo;a&rsquo;&ldquo;b&rdquo;&frac12;&uarr;&darr;&sum;&radic;&part;&int;&sim;&cong;&Aring;</Root>";
        let root = parse_xml(xml).unwrap();
        assert_eq!(
            root.plain_text(),
            "\u{00A9}\u{00AE}\u{2122}\u{2018}a\u{2019}\u{201C}b\u{201D}\u{00BD}\u{2191}\u{2193}\u{2211}\u{221A}\u{2202}\u{222B}\u{223C}\u{2245}\u{00C5}"
        );
    }

    #[test]
    fn quick_xml_adapter_rejects_malformed_declarations_and_enforces_depth() {
        let malformed = [
            "",
            "<!-- only comment -->",
            "text<Root/>",
            "<A/><B/>",
            "<A></B>",
            "<A>",
            "<A>&unknown;</A>",
            "<A>bare & amp</A>",
            "<!DOCTYPE a><A/>",
            "<!ENTITY x \"y\"><A/>",
            "<A><!ENTITY x \"y\"/></A>",
            "<A><![CDATA[unclosed</A>",
            "<A attr=unquoted/>",
            "<A attr=\"unclosed/>",
        ];
        for sample in malformed {
            assert!(matches!(parse_xml(sample), Err(FormatError::XmlParse { .. })), "expected XmlParse error for `{sample}`");
        }
        assert!(matches!(
            parse_xml_with_limits("<A><B/></A>", XmlLimits { max_bytes: 1024, max_depth: 1, max_nodes: 100 }),
            Err(FormatError::XmlDepthExceeded { depth: 2, max: 1 })
        ));
    }
}
