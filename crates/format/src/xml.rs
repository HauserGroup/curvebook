//! Bounded XML DOM parser backed by `quick-xml`'s event reader for clean-room scientific XML
//! interchange.
//!
//! Enforces explicit byte, nesting-depth, and node-count caps, strips UTF-8 BOM, preserves
//! ordered mixed content, handles comments, CDATA sections, numeric character references, and
//! scientific named character entities, and rejects `<!DOCTYPE` / `<!ENTITY` declarations (zero
//! external entity expansion).

use std::collections::HashSet;

use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};

use crate::error::FormatError;

/// Default maximum XML payload size in bytes (64 MiB).
pub const MAX_XML_BYTES: usize = 64 * 1024 * 1024;

/// Default maximum XML element nesting depth.
pub const MAX_XML_DEPTH: usize = 128;

/// Default maximum number of XML elements in a single document.
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
        self.name.rsplit_once(':').map_or(self.name.as_str(), |(_, local)| local)
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
    if input.len() > limits.max_bytes {
        return Err(FormatError::XmlParse {
            offset: limits.max_bytes,
            reason: format!("XML byte length {} exceeds limit {}", input.len(), limits.max_bytes),
        });
    }
    let src = input.strip_prefix('\u{FEFF}').unwrap_or(input);
    let mut builder = XmlTreeBuilder::new(limits);
    builder.parse(src)
}

struct XmlTreeBuilder {
    limits: XmlLimits,
    node_count: usize,
    stack: Vec<XmlElement>,
    pending_text: String,
    root: Option<XmlElement>,
}

impl XmlTreeBuilder {
    fn new(limits: XmlLimits) -> Self {
        Self { limits, node_count: 0, stack: Vec::new(), pending_text: String::new(), root: None }
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
            self.bump_node_count(offset, "text node")?;
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

        let name_bytes = start.name();
        let name = std::str::from_utf8(name_bytes.as_ref())
            .map_err(|err| FormatError::XmlParse { offset, reason: format!("invalid UTF-8 in XML tag name: {err}") })?;
        if name.is_empty() {
            return Err(FormatError::XmlParse { offset, reason: String::from("empty XML tag name") });
        }

        let attrs = self.read_attributes(start, offset)?;
        Ok(XmlElement { name: name.to_owned(), attrs, children: Vec::new() })
    }

    fn read_attributes(&mut self, start: &BytesStart<'_>, offset: usize) -> Result<Vec<(String, String)>, FormatError> {
        let mut attrs: Vec<(String, String)> = Vec::new();
        let mut seen: Option<HashSet<String>> = None;

        for attr_res in start.attributes().with_checks(false) {
            let attr = attr_res.map_err(|err| FormatError::XmlParse { offset, reason: format!("malformed XML attribute: {err}") })?;
            self.bump_node_count(offset, "attribute")?;
            let key = std::str::from_utf8(attr.key.as_ref())
                .map_err(|err| FormatError::XmlParse { offset, reason: format!("invalid UTF-8 in attribute name: {err}") })?;
            if key.is_empty() {
                return Err(FormatError::XmlParse { offset, reason: String::from("expected attribute name") });
            }
            if is_duplicate_attr(&attrs, &mut seen, key) {
                return Err(FormatError::XmlParse { offset, reason: format!("duplicate attribute `{key}`") });
            }
            let raw_val = std::str::from_utf8(attr.value.as_ref())
                .map_err(|err| FormatError::XmlParse { offset, reason: format!("invalid UTF-8 in attribute value: {err}") })?;
            let decoded = decode_xml_entities(raw_val, offset)?;
            attrs.push((key.to_owned(), decoded));
        }

        Ok(attrs)
    }

    fn attach_completed_element(&mut self, element: XmlElement) {
        if let Some(parent) = self.stack.last_mut() {
            parent.children.push(XmlNode::Element(element));
        } else {
            self.root = Some(element);
        }
    }

    fn handle_text_bytes(&mut self, raw_bytes: &[u8], offset: usize) -> Result<(), FormatError> {
        let raw = std::str::from_utf8(raw_bytes).map_err(|err| FormatError::XmlParse { offset, reason: format!("invalid UTF-8 in text: {err}") })?;
        if self.stack.is_empty() {
            if !raw.trim().is_empty() {
                return Err(FormatError::XmlParse { offset, reason: String::from("unexpected character data outside root element") });
            }
            return Ok(());
        }
        let decoded = decode_xml_entities(raw, offset)?;
        self.pending_text.push_str(&decoded);
        Ok(())
    }

    fn handle_general_ref(&mut self, raw_bytes: &[u8], offset: usize) -> Result<(), FormatError> {
        if self.stack.is_empty() {
            return Err(FormatError::XmlParse { offset, reason: String::from("unexpected character data outside root element") });
        }
        let entity =
            std::str::from_utf8(raw_bytes).map_err(|err| FormatError::XmlParse { offset, reason: format!("invalid UTF-8 in entity: {err}") })?;
        let ch = decode_xml_entity_name(entity, offset)?;
        self.pending_text.push(ch);
        Ok(())
    }

    fn handle_cdata(&mut self, raw_bytes: &[u8], offset: usize) -> Result<(), FormatError> {
        if self.stack.is_empty() {
            return Err(FormatError::XmlParse { offset, reason: String::from("unexpected CDATA outside root element") });
        }
        self.flush_pending_text(offset)?;
        let cdata =
            std::str::from_utf8(raw_bytes).map_err(|err| FormatError::XmlParse { offset, reason: format!("invalid UTF-8 in CDATA: {err}") })?;
        if !cdata.is_empty() {
            self.bump_node_count(offset, "CDATA node")?;
            if let Some(parent) = self.stack.last_mut() {
                parent.children.push(XmlNode::Text(cdata.to_owned()));
            }
        }
        Ok(())
    }

    fn handle_event(&mut self, event: Event<'_>, offset: usize) -> Result<bool, FormatError> {
        match event {
            Event::Start(start) => {
                self.flush_pending_text(offset)?;
                let el = self.build_element(&start, offset)?;
                self.stack.push(el);
            }
            Event::Empty(start) => {
                self.flush_pending_text(offset)?;
                let el = self.build_element(&start, offset)?;
                self.attach_completed_element(el);
            }
            Event::End(end) => {
                self.flush_pending_text(offset)?;
                let close_qname = end.name();
                let close_name = std::str::from_utf8(close_qname.as_ref())
                    .map_err(|err| FormatError::XmlParse { offset, reason: format!("invalid UTF-8 in closing tag: {err}") })?;
                let Some(el) = self.stack.pop() else {
                    return Err(FormatError::XmlParse { offset, reason: format!("unexpected closing tag `</{close_name}>`") });
                };
                if el.name != close_name {
                    return Err(FormatError::XmlParse {
                        offset,
                        reason: format!("mismatched closing tag `</{close_name}>`, expected `</{}>`", el.name),
                    });
                }
                self.attach_completed_element(el);
            }
            Event::Text(text) => self.handle_text_bytes(text.as_ref(), offset)?,
            Event::GeneralRef(entity) => self.handle_general_ref(entity.as_ref(), offset)?,
            Event::CData(cdata) => self.handle_cdata(cdata.as_ref(), offset)?,
            Event::Comment(_) | Event::PI(_) | Event::Decl(_) => {
                self.flush_pending_text(offset)?;
            }
            Event::DocType(_) => {
                return Err(FormatError::XmlParse { offset, reason: String::from("DOCTYPE and external ENTITY declarations are forbidden") });
            }
            Event::Eof => return Ok(true),
        }
        Ok(false)
    }

    fn parse(&mut self, src: &str) -> Result<XmlElement, FormatError> {
        let mut reader = Reader::from_str(src);
        reader.config_mut().check_end_names = true;
        reader.config_mut().trim_text(false);

        loop {
            let offset = usize::try_from(reader.buffer_position()).unwrap_or(usize::MAX);
            let event = reader.read_event().map_err(|err| FormatError::XmlParse { offset, reason: format!("malformed XML: {err}") })?;
            if self.handle_event(event, offset)? {
                break;
            }
        }

        if let Some(unclosed) = self.stack.last() {
            let offset = usize::try_from(reader.buffer_position()).unwrap_or(usize::MAX);
            return Err(FormatError::XmlParse { offset, reason: format!("unclosed XML tag `<{}>`", unclosed.name) });
        }
        self.root.take().ok_or_else(|| FormatError::XmlParse {
            offset: usize::try_from(reader.buffer_position()).unwrap_or(usize::MAX),
            reason: String::from("empty XML document"),
        })
    }
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

/// Decode standard XML character entities (`&lt;`, `&gt;`, `&amp;`, `&apos;`, `&quot;`), scientific
/// named entities, and numeric character references (`&#123;`, `&#x1F;`).
pub fn decode_xml_entities(raw: &str, base_offset: usize) -> Result<String, FormatError> {
    if !raw.contains('&') {
        return Ok(raw.to_owned());
    }
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some((before, after_amp)) = rest.split_once('&') {
        out.push_str(before);
        let Some((entity, after_semi)) = after_amp.split_once(';') else {
            return Err(FormatError::XmlParse { offset: base_offset, reason: String::from("unterminated XML entity reference") });
        };
        out.push(decode_xml_entity_name(entity, base_offset)?);
        rest = after_semi;
    }
    out.push_str(rest);
    Ok(out)
}

fn decode_xml_entity_name(entity: &str, base_offset: usize) -> Result<char, FormatError> {
    if let Some(ch) = lookup_named_xml_entity(entity) {
        Ok(ch)
    } else if let Some(hex) = entity.strip_prefix("#x").or_else(|| entity.strip_prefix("#X")) {
        let code = u32::from_str_radix(hex, 16)
            .map_err(|_| FormatError::XmlParse { offset: base_offset, reason: format!("invalid hex XML entity `&{entity};`") })?;
        char::from_u32(code).ok_or_else(|| FormatError::XmlParse { offset: base_offset, reason: format!("invalid Unicode scalar `&{entity};`") })
    } else if let Some(digits) = entity.strip_prefix('#') {
        let code = digits
            .parse::<u32>()
            .map_err(|_| FormatError::XmlParse { offset: base_offset, reason: format!("invalid decimal XML entity `&{entity};`") })?;
        char::from_u32(code).ok_or_else(|| FormatError::XmlParse { offset: base_offset, reason: format!("invalid Unicode scalar `&{entity};`") })
    } else {
        Err(FormatError::XmlParse { offset: base_offset, reason: format!("unknown XML entity `&{entity};`") })
    }
}

fn lookup_named_xml_entity(entity: &str) -> Option<char> {
    match entity {
        "lt" => Some('<'),
        "gt" => Some('>'),
        "amp" => Some('&'),
        "apos" => Some('\''),
        "quot" => Some('"'),
        "nbsp" => Some('\u{00A0}'),
        "alpha" => Some('\u{03B1}'),
        "beta" => Some('\u{03B2}'),
        "gamma" => Some('\u{03B3}'),
        "delta" => Some('\u{03B4}'),
        "epsilon" => Some('\u{03B5}'),
        "varepsilon" => Some('\u{03F5}'),
        "zeta" => Some('\u{03B6}'),
        "eta" => Some('\u{03B7}'),
        "theta" => Some('\u{03B8}'),
        "vartheta" => Some('\u{03D1}'),
        "iota" => Some('\u{03B9}'),
        "kappa" => Some('\u{03BA}'),
        "varkappa" => Some('\u{03F0}'),
        "lambda" => Some('\u{03BB}'),
        "mu" | "micro" => Some('\u{03BC}'),
        "nu" => Some('\u{03BD}'),
        "xi" => Some('\u{03BE}'),
        "omicron" => Some('\u{03BF}'),
        "pi" => Some('\u{03C0}'),
        "varpi" => Some('\u{03D6}'),
        "rho" => Some('\u{03C1}'),
        "varrho" => Some('\u{03F1}'),
        "varsigma" => Some('\u{03C2}'),
        "sigma" => Some('\u{03C3}'),
        "tau" => Some('\u{03C4}'),
        "upsilon" => Some('\u{03C5}'),
        "phi" => Some('\u{03C6}'),
        "varphi" => Some('\u{03D5}'),
        "chi" => Some('\u{03C7}'),
        "psi" => Some('\u{03C8}'),
        "omega" => Some('\u{03C9}'),
        "Alpha" => Some('\u{0391}'),
        "Beta" => Some('\u{0392}'),
        "Gamma" => Some('\u{0393}'),
        "Delta" => Some('\u{0394}'),
        "Epsilon" => Some('\u{0395}'),
        "Zeta" => Some('\u{0396}'),
        "Eta" => Some('\u{0397}'),
        "Theta" => Some('\u{0398}'),
        "Iota" => Some('\u{0399}'),
        "Kappa" => Some('\u{039A}'),
        "Lambda" => Some('\u{039B}'),
        "Mu" => Some('\u{039C}'),
        "Nu" => Some('\u{039D}'),
        "Xi" => Some('\u{039E}'),
        "Omicron" => Some('\u{039F}'),
        "Pi" => Some('\u{03A0}'),
        "Rho" => Some('\u{03A1}'),
        "Sigma" => Some('\u{03A3}'),
        "Tau" => Some('\u{03A4}'),
        "Upsilon" => Some('\u{03A5}'),
        "Phi" => Some('\u{03A6}'),
        "Chi" => Some('\u{03A7}'),
        "Psi" => Some('\u{03A8}'),
        "Omega" | "ohm" => Some('\u{03A9}'),
        "deg" => Some('\u{00B0}'),
        "pm" | "plusmn" => Some('\u{00B1}'),
        "mp" => Some('\u{2213}'),
        "sup1" => Some('\u{00B9}'),
        "sup2" => Some('\u{00B2}'),
        "sup3" => Some('\u{00B3}'),
        "times" => Some('\u{00D7}'),
        "divide" | "div" => Some('\u{00F7}'),
        "cdot" | "middot" => Some('\u{00B7}'),
        "bull" | "bullet" => Some('\u{2022}'),
        "minus" => Some('\u{2212}'),
        "ndash" => Some('\u{2013}'),
        "mdash" => Some('\u{2014}'),
        "hellip" => Some('\u{2026}'),
        "prime" => Some('\u{2032}'),
        "Prime" => Some('\u{2033}'),
        "permil" => Some('\u{2030}'),
        "infin" => Some('\u{221E}'),
        "le" | "leq" => Some('\u{2264}'),
        "ge" | "geq" => Some('\u{2265}'),
        "ne" | "neq" => Some('\u{2260}'),
        "asymp" | "approx" => Some('\u{2248}'),
        "equiv" => Some('\u{2261}'),
        "rarr" => Some('\u{2192}'),
        "larr" => Some('\u{2190}'),
        "harr" => Some('\u{2194}'),
        _ => None,
    }
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
