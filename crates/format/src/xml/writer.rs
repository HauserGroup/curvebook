//! Deterministic, strict XML 1.0 writer.
//!
//! Preserves caller-controlled attribute order, escapes `& < > " '` (plus whitespace control
//! preservation for `\t`, `\n`, `\r` in attribute values and `\r` in text), validates element and
//! attribute names against the W3C XML 1.0 Fifth Edition `Name` production, and rejects any
//! character outside the XML 1.0 §2.2 `Char` production with a typed [`FormatError`] so invalid
//! XML is never silently emitted.

use std::collections::HashSet;

use super::entities::is_valid_xml10_char;
use super::{XmlElement, XmlNode};
use crate::error::FormatError;

/// Deterministic XML 1.0 writer with caller-controlled attribute order and strict validation.
#[derive(Debug, Clone)]
pub struct XmlWriter {
    buf: String,
    open_elements: Vec<OpenElementState>,
    indent: Option<String>,
    wrote_root: bool,
    inline_depth: usize,
}

#[derive(Debug, Clone)]
struct OpenElementState {
    name: String,
    has_child_elements: bool,
    has_text: bool,
    started_inline: bool,
}

impl Default for XmlWriter {
    fn default() -> Self {
        Self::new()
    }
}

impl XmlWriter {
    /// Create a compact (non-indented) deterministic XML writer.
    #[must_use]
    pub fn new() -> Self {
        Self { buf: String::new(), open_elements: Vec::new(), indent: None, wrote_root: false, inline_depth: 0 }
    }

    /// Create a deterministic pretty-printing XML writer using `indent` per nesting level.
    ///
    /// `indent` must contain only XML whitespace (`' '` or `'\t'`). Mixed-content elements can
    /// preserve exact inline text spacing via [`Self::start_inline_element`] /
    /// [`Self::empty_inline_element`].
    pub fn with_indent(indent: impl Into<String>) -> Result<Self, FormatError> {
        let indent_str = indent.into();
        if !indent_str.bytes().all(|b| matches!(b, b' ' | b'\t')) {
            return Err(FormatError::XmlParse { offset: 0, reason: String::from("XML writer indentation must consist only of spaces or tabs") });
        }
        Ok(Self { buf: String::new(), open_elements: Vec::new(), indent: Some(indent_str), wrote_root: false, inline_depth: 0 })
    }

    /// Write the canonical UTF-8 XML 1.0 declaration (`<?xml version="1.0" encoding="UTF-8"?>`).
    /// Must be called before any root element or other markup.
    pub fn write_xml_declaration(&mut self) -> Result<(), FormatError> {
        if !self.buf.is_empty() {
            return Err(FormatError::XmlParse {
                offset: self.buf.len(),
                reason: String::from("XML declaration must appear at the start of the document"),
            });
        }
        self.buf.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>");
        if self.indent.is_some() {
            self.buf.push('\n');
        }
        Ok(())
    }

    /// Open a new element with attributes emitted in the exact order given by `attrs`.
    pub fn start_element(&mut self, name: &str, attrs: &[(&str, &str)]) -> Result<(), FormatError> {
        self.start_element_inner(name, attrs, false)
    }

    /// Open an inline element (suppressing indentation inside its subtree for mixed content such
    /// as rich-text `<b>` / `<sub>` / `<sup>` runs).
    pub fn start_inline_element(&mut self, name: &str, attrs: &[(&str, &str)]) -> Result<(), FormatError> {
        self.start_element_inner(name, attrs, true)
    }

    /// Write a self-closing empty element (`<Name .../>`) with caller-ordered attributes.
    pub fn empty_element(&mut self, name: &str, attrs: &[(&str, &str)]) -> Result<(), FormatError> {
        self.empty_element_inner(name, attrs, false)
    }

    /// Write an inline self-closing empty element without surrounding pretty-print newlines.
    pub fn empty_inline_element(&mut self, name: &str, attrs: &[(&str, &str)]) -> Result<(), FormatError> {
        self.empty_element_inner(name, attrs, true)
    }

    /// Write an element containing a single escaped text body (`<Name ...>text</Name>`).
    pub fn text_element(&mut self, name: &str, attrs: &[(&str, &str)], text: &str) -> Result<(), FormatError> {
        self.validate_element_open(name, attrs)?;
        validate_xml10_text(text, self.buf.len())?;

        self.write_child_prefix(false);
        self.mark_root_or_child_element();
        self.write_open_tag_head(name, attrs);
        self.buf.push('>');
        push_escaped_xml_text(&mut self.buf, text);
        self.buf.push_str("</");
        self.buf.push_str(name);
        self.buf.push('>');
        Ok(())
    }

    /// Write escaped character data inside the currently open element.
    pub fn text(&mut self, content: &str) -> Result<(), FormatError> {
        if self.open_elements.is_empty() {
            return Err(FormatError::XmlParse { offset: self.buf.len(), reason: String::from("cannot write character data outside a root element") });
        }
        validate_xml10_text(content, self.buf.len())?;
        if let Some(parent) = self.open_elements.last_mut() {
            parent.has_text = true;
        }
        push_escaped_xml_text(&mut self.buf, content);
        Ok(())
    }

    /// Write a CDATA section (`<![CDATA[...]]>`), rejecting invalid XML 1.0 characters and the
    /// forbidden `]]>` delimiter sequence.
    pub fn cdata(&mut self, content: &str) -> Result<(), FormatError> {
        if self.open_elements.is_empty() {
            return Err(FormatError::XmlParse { offset: self.buf.len(), reason: String::from("cannot write CDATA outside a root element") });
        }
        validate_xml10_text(content, self.buf.len())?;
        if content.contains("]]>") {
            return Err(FormatError::XmlParse { offset: self.buf.len(), reason: String::from("CDATA content cannot contain `]]>`") });
        }
        if let Some(parent) = self.open_elements.last_mut() {
            parent.has_text = true;
        }
        self.buf.push_str("<![CDATA[");
        self.buf.push_str(content);
        self.buf.push_str("]]>");
        Ok(())
    }

    /// Write an XML comment (`<!--...-->`), rejecting invalid XML 1.0 characters, `--`, and a
    /// trailing `-` per XML 1.0 §2.5.
    pub fn comment(&mut self, content: &str) -> Result<(), FormatError> {
        validate_xml10_text(content, self.buf.len())?;
        if content.contains("--") || content.ends_with('-') {
            return Err(FormatError::XmlParse { offset: self.buf.len(), reason: String::from("XML comment cannot contain `--` or end with `-`") });
        }
        self.write_child_prefix(false);
        self.buf.push_str("<!--");
        self.buf.push_str(content);
        self.buf.push_str("-->");
        if self.open_elements.is_empty() && self.indent.is_some() {
            self.buf.push('\n');
        }
        Ok(())
    }

    /// Close the innermost open element.
    pub fn end_element(&mut self) -> Result<(), FormatError> {
        let Some(state) = self.open_elements.pop() else {
            return Err(FormatError::XmlParse { offset: self.buf.len(), reason: String::from("end_element called with no open XML element") });
        };
        if state.started_inline {
            self.inline_depth = self.inline_depth.saturating_sub(1);
        }
        if self.inline_depth == 0 && state.has_child_elements && !state.has_text {
            self.push_newline_and_indent(self.open_elements.len());
        }
        self.buf.push_str("</");
        self.buf.push_str(&state.name);
        self.buf.push('>');
        Ok(())
    }

    /// Close the innermost open element after verifying its name matches `expected_name`.
    pub fn end_element_named(&mut self, expected_name: &str) -> Result<(), FormatError> {
        let Some(top) = self.open_elements.last() else {
            return Err(FormatError::XmlParse {
                offset: self.buf.len(),
                reason: format!("end_element_named(`{expected_name}`) called with no open XML element"),
            });
        };
        if top.name != expected_name {
            return Err(FormatError::XmlParse {
                offset: self.buf.len(),
                reason: format!("mismatched end_element_named(`{expected_name}`), open element is `{}`", top.name),
            });
        }
        self.end_element()
    }

    /// Deterministically serialize an [`XmlElement`] tree, preserving exact attribute and
    /// mixed-content child ordering.
    pub fn write_element(&mut self, element: &XmlElement) -> Result<(), FormatError> {
        let attrs: Vec<(&str, &str)> = element.attrs.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        if element.children.is_empty() {
            return self.empty_element(&element.name, &attrs);
        }
        let has_mixed_text = element.children.iter().any(|n| matches!(n, XmlNode::Text(_)));
        if has_mixed_text {
            self.start_inline_element(&element.name, &attrs)?;
        } else {
            self.start_element(&element.name, &attrs)?;
        }
        for child in &element.children {
            match child {
                XmlNode::Text(text) => self.text(text)?,
                XmlNode::Element(el) => self.write_element(el)?,
            }
        }
        self.end_element()
    }

    /// Finish writing and return the deterministic XML string.
    /// Fails if any elements remain unclosed or if no root element was written.
    pub fn finish(self) -> Result<String, FormatError> {
        if let Some(unclosed) = self.open_elements.last() {
            return Err(FormatError::XmlParse { offset: self.buf.len(), reason: format!("unclosed XML element `<{}>` at finish", unclosed.name) });
        }
        if !self.wrote_root {
            return Err(FormatError::XmlParse { offset: self.buf.len(), reason: String::from("cannot finish XML document without a root element") });
        }
        Ok(self.buf)
    }

    /// Finish writing and return UTF-8 bytes.
    pub fn finish_bytes(self) -> Result<Vec<u8>, FormatError> {
        self.finish().map(String::into_bytes)
    }

    fn start_element_inner(&mut self, name: &str, attrs: &[(&str, &str)], inline: bool) -> Result<(), FormatError> {
        self.validate_element_open(name, attrs)?;
        self.write_child_prefix(inline);
        self.mark_root_or_child_element();
        self.write_open_tag_head(name, attrs);
        self.buf.push('>');
        if inline {
            self.inline_depth += 1;
        }
        self.open_elements.push(OpenElementState { name: name.to_owned(), has_child_elements: false, has_text: false, started_inline: inline });
        Ok(())
    }

    fn empty_element_inner(&mut self, name: &str, attrs: &[(&str, &str)], inline: bool) -> Result<(), FormatError> {
        self.validate_element_open(name, attrs)?;
        self.write_child_prefix(inline);
        self.mark_root_or_child_element();
        self.write_open_tag_head(name, attrs);
        self.buf.push_str("/>");
        Ok(())
    }

    fn validate_element_open(&self, name: &str, attrs: &[(&str, &str)]) -> Result<(), FormatError> {
        if self.open_elements.is_empty() && self.wrote_root {
            return Err(FormatError::XmlParse { offset: self.buf.len(), reason: String::from("cannot write multiple root XML elements") });
        }
        validate_xml10_name(name, self.buf.len(), "element")?;
        validate_attrs(attrs, self.buf.len())
    }

    fn mark_root_or_child_element(&mut self) {
        if let Some(parent) = self.open_elements.last_mut() {
            parent.has_child_elements = true;
        } else {
            self.wrote_root = true;
        }
    }

    fn write_child_prefix(&mut self, inline: bool) {
        if inline || self.inline_depth > 0 || self.indent.is_none() {
            return;
        }
        if let Some(parent) = self.open_elements.last()
            && !parent.has_text
        {
            self.push_newline_and_indent(self.open_elements.len());
        }
    }

    fn push_newline_and_indent(&mut self, depth: usize) {
        let Some(indent) = self.indent.as_deref() else {
            return;
        };
        self.buf.push('\n');
        for _ in 0..depth {
            self.buf.push_str(indent);
        }
    }

    fn write_open_tag_head(&mut self, name: &str, attrs: &[(&str, &str)]) {
        self.buf.push('<');
        self.buf.push_str(name);
        for &(key, value) in attrs {
            self.buf.push(' ');
            self.buf.push_str(key);
            self.buf.push_str("=\"");
            push_escaped_xml_attr(&mut self.buf, value);
            self.buf.push('"');
        }
    }
}

fn validate_attrs(attrs: &[(&str, &str)], offset: usize) -> Result<(), FormatError> {
    let mut seen: Option<HashSet<&str>> = None;
    for (idx, &(key, val)) in attrs.iter().enumerate() {
        validate_xml10_name(key, offset, "attribute")?;
        validate_xml10_text(val, offset)?;
        if is_duplicate_attr_slice(attrs.get(..idx).unwrap_or_default(), &mut seen, key) {
            return Err(FormatError::XmlParse { offset, reason: format!("duplicate XML attribute `{key}`") });
        }
    }
    Ok(())
}

fn is_duplicate_attr_slice<'a>(prior: &[(&'a str, &'a str)], seen: &mut Option<HashSet<&'a str>>, key: &'a str) -> bool {
    if let Some(set) = seen.as_mut() {
        return !set.insert(key);
    }
    if prior.iter().any(|&(existing, _)| existing == key) {
        return true;
    }
    if prior.len() >= 16 {
        let mut set: HashSet<&'a str> = prior.iter().map(|&(k, _)| k).collect();
        set.insert(key);
        *seen = Some(set);
    }
    false
}

fn validate_xml10_text(text: &str, offset: usize) -> Result<(), FormatError> {
    for ch in text.chars() {
        if !is_valid_xml10_char(ch) {
            return Err(FormatError::XmlParse { offset, reason: format!("forbidden XML 1.0 character U+{:04X}", ch as u32) });
        }
    }
    Ok(())
}

fn validate_xml10_name(name: &str, offset: usize, kind: &str) -> Result<(), FormatError> {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return Err(FormatError::XmlParse { offset, reason: format!("empty XML {kind} name") });
    };
    if !is_xml10_name_start_char(first) {
        return Err(FormatError::XmlParse { offset, reason: format!("invalid XML {kind} name `{name}`") });
    }
    if !chars.all(is_xml10_name_char) {
        return Err(FormatError::XmlParse { offset, reason: format!("invalid XML {kind} name `{name}`") });
    }
    Ok(())
}

/// W3C XML 1.0 Fifth Edition §2.3 `NameStartChar`.
const fn is_xml10_name_start_char(ch: char) -> bool {
    matches!(
        ch as u32,
        0x3A
            | 0x41..=0x5A
            | 0x5F
            | 0x61..=0x7A
            | 0xC0..=0xD6
            | 0xD8..=0xF6
            | 0xF8..=0x02FF
            | 0x0370..=0x037D
            | 0x037F..=0x1FFF
            | 0x200C..=0x200D
            | 0x2070..=0x218F
            | 0x2C00..=0x2FEF
            | 0x3001..=0xD7FF
            | 0xF900..=0xFDCF
            | 0xFDF0..=0xFFFD
            | 0x10000..=0xEFFFF
    )
}

/// W3C XML 1.0 Fifth Edition §2.3 `NameChar`.
const fn is_xml10_name_char(ch: char) -> bool {
    is_xml10_name_start_char(ch) || matches!(ch as u32, 0x2D | 0x2E | 0x30..=0x39 | 0xB7 | 0x0300..=0x036F | 0x203F..=0x2040)
}

fn push_escaped_xml_text(out: &mut String, text: &str) {
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\r' => out.push_str("&#xD;"),
            other => out.push(other),
        }
    }
}

fn push_escaped_xml_attr(out: &mut String, value: &str) {
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\t' => out.push_str("&#x9;"),
            '\n' => out.push_str("&#xA;"),
            '\r' => out.push_str("&#xD;"),
            other => out.push(other),
        }
    }
}
