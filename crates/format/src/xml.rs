//! Bounded pure-`std` XML DOM parser for clean-room scientific XML interchange.
//!
//! Enforces explicit byte, nesting-depth, and node-count caps, strips UTF-8 BOM, handles
//! comments, CDATA sections, and standard character entities, and rejects `<!DOCTYPE` /
//! `<!ENTITY` declarations (zero external entity expansion).

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
    let mut parser = XmlParser { src, pos: 0, node_count: 0, limits };
    parser.parse_document()
}

struct XmlParser<'a> {
    src: &'a str,
    pos: usize,
    node_count: usize,
    limits: XmlLimits,
}

impl<'a> XmlParser<'a> {
    fn rest(&self) -> &'a str {
        &self.src[self.pos..]
    }

    fn skip_whitespace(&mut self) {
        let trimmed = self.rest().trim_start();
        self.pos = self.src.len() - trimmed.len();
    }

    fn parse_document(&mut self) -> Result<XmlElement, FormatError> {
        let mut root: Option<XmlElement> = None;
        loop {
            self.skip_whitespace();
            if self.pos >= self.src.len() {
                break;
            }
            if !self.rest().starts_with('<') {
                return Err(FormatError::XmlParse { offset: self.pos, reason: String::from("unexpected character data outside root element") });
            }
            if self.skip_misc()? {
                continue;
            }
            if root.is_some() {
                return Err(FormatError::XmlParse { offset: self.pos, reason: String::from("multiple root XML elements") });
            }
            root = Some(self.parse_element(1)?);
        }
        root.ok_or_else(|| FormatError::XmlParse { offset: self.pos, reason: String::from("empty XML document") })
    }

    fn skip_misc(&mut self) -> Result<bool, FormatError> {
        let r = self.rest();
        if r.starts_with("<?") {
            let end =
                r.find("?>").ok_or_else(|| FormatError::XmlParse { offset: self.pos, reason: String::from("unclosed processing instruction") })?;
            self.pos += end + 2;
            return Ok(true);
        }
        if r.starts_with("<!--") {
            let end = r.find("-->").ok_or_else(|| FormatError::XmlParse { offset: self.pos, reason: String::from("unclosed XML comment") })?;
            self.pos += end + 3;
            return Ok(true);
        }
        if r.starts_with("<!DOCTYPE") || r.starts_with("<!ENTITY") {
            return Err(FormatError::XmlParse { offset: self.pos, reason: String::from("DOCTYPE and external ENTITY declarations are forbidden") });
        }
        Ok(false)
    }

    fn bump_node_count(&mut self, kind: &str) -> Result<(), FormatError> {
        self.node_count += 1;
        if self.node_count > self.limits.max_nodes {
            return Err(FormatError::XmlParse { offset: self.pos, reason: format!("XML {kind} count exceeds node limit {}", self.limits.max_nodes) });
        }
        Ok(())
    }

    fn parse_element(&mut self, depth: usize) -> Result<XmlElement, FormatError> {
        if depth > self.limits.max_depth {
            return Err(FormatError::XmlDepthExceeded { depth, max: self.limits.max_depth });
        }
        self.bump_node_count("element")?;
        if !self.rest().starts_with('<') {
            return Err(FormatError::XmlParse { offset: self.pos, reason: String::from("expected `<` opening tag") });
        }
        self.pos += 1;
        let name = self.read_name()?;
        if name.is_empty() {
            return Err(FormatError::XmlParse { offset: self.pos, reason: String::from("empty XML tag name") });
        }
        let attrs = self.read_attributes()?;

        if self.rest().starts_with("/>") {
            self.pos += 2;
            return Ok(XmlElement { name, attrs, children: Vec::new() });
        }
        if !self.rest().starts_with('>') {
            return Err(FormatError::XmlParse { offset: self.pos, reason: String::from("expected `>` after start tag") });
        }
        self.pos += 1;

        let children = self.parse_children(&name, depth)?;
        Ok(XmlElement { name, attrs, children })
    }

    fn parse_children(&mut self, tag_name: &str, depth: usize) -> Result<Vec<XmlNode>, FormatError> {
        let mut children = Vec::new();
        loop {
            if self.pos >= self.src.len() {
                return Err(FormatError::XmlParse { offset: self.pos, reason: format!("unclosed XML tag `<{tag_name}>`") });
            }
            if self.rest().starts_with("</") {
                self.pos += 2;
                let close_name = self.read_name()?;
                self.skip_whitespace();
                if !self.rest().starts_with('>') {
                    return Err(FormatError::XmlParse { offset: self.pos, reason: String::from("expected `>` in closing tag") });
                }
                self.pos += 1;
                if close_name != tag_name {
                    return Err(FormatError::XmlParse {
                        offset: self.pos,
                        reason: format!("mismatched closing tag `</{close_name}>`, expected `</{tag_name}>`"),
                    });
                }
                return Ok(children);
            }
            if self.rest().starts_with("<![CDATA[") {
                self.pos += 9;
                let end =
                    self.rest().find("]]>").ok_or_else(|| FormatError::XmlParse { offset: self.pos, reason: String::from("unclosed CDATA") })?;
                let cdata = &self.rest()[..end];
                self.pos += end + 3;
                if !cdata.is_empty() {
                    self.bump_node_count("CDATA node")?;
                    children.push(XmlNode::Text(cdata.to_owned()));
                }
                continue;
            }
            if self.rest().starts_with('<') {
                if self.skip_misc()? {
                    continue;
                }
                children.push(XmlNode::Element(self.parse_element(depth + 1)?));
            } else {
                let text = self.read_text_until_lt()?;
                if !text.is_empty() {
                    self.bump_node_count("text node")?;
                    children.push(XmlNode::Text(text));
                }
            }
        }
    }

    fn read_name(&mut self) -> Result<String, FormatError> {
        let start = self.pos;
        for (idx, ch) in self.rest().char_indices() {
            if is_name_char(ch) {
                continue;
            }
            self.pos += idx;
            return Ok(self.src[start..self.pos].to_owned());
        }
        Err(FormatError::XmlParse { offset: self.pos, reason: String::from("unexpected EOF in XML name") })
    }

    fn read_attributes(&mut self) -> Result<Vec<(String, String)>, FormatError> {
        let mut attrs: Vec<(String, String)> = Vec::new();
        let mut seen: Option<std::collections::HashSet<String>> = None;
        loop {
            self.skip_whitespace();
            let r = self.rest();
            if r.starts_with("/>") || r.starts_with('>') {
                return Ok(attrs);
            }
            if r.is_empty() {
                return Err(FormatError::XmlParse { offset: self.pos, reason: String::from("unexpected EOF in tag attributes") });
            }
            self.bump_node_count("attribute")?;
            let attr_offset = self.pos;
            let key = self.read_name()?;
            if key.is_empty() {
                return Err(FormatError::XmlParse { offset: self.pos, reason: String::from("expected attribute name") });
            }
            self.skip_whitespace();
            if !self.rest().starts_with('=') {
                return Err(FormatError::XmlParse { offset: self.pos, reason: format!("expected `=` after attribute `{key}`") });
            }
            self.pos += 1;
            self.skip_whitespace();
            let quote = self
                .rest()
                .chars()
                .next()
                .ok_or_else(|| FormatError::XmlParse { offset: self.pos, reason: String::from("unexpected EOF before attribute value") })?;
            if quote != '"' && quote != '\'' {
                return Err(FormatError::XmlParse { offset: self.pos, reason: String::from("attribute value must be quoted") });
            }
            self.pos += 1;
            let end = self
                .rest()
                .find(quote)
                .ok_or_else(|| FormatError::XmlParse { offset: self.pos, reason: String::from("unclosed attribute quote") })?;
            let raw_val = &self.rest()[..end];
            let decoded = decode_xml_entities(raw_val, self.pos)?;
            self.pos += end + 1;
            if Self::is_duplicate_attr(&attrs, &mut seen, &key) {
                return Err(FormatError::XmlParse { offset: attr_offset, reason: format!("duplicate attribute `{key}`") });
            }
            attrs.push((key, decoded));
        }
    }

    fn is_duplicate_attr(attrs: &[(String, String)], seen: &mut Option<std::collections::HashSet<String>>, key: &str) -> bool {
        if let Some(set) = seen.as_mut() {
            return !set.insert(key.to_owned());
        }
        if attrs.iter().any(|(existing, _)| existing == key) {
            return true;
        }
        if attrs.len() >= 16 {
            let mut set: std::collections::HashSet<String> = attrs.iter().map(|(k, _)| k.clone()).collect();
            set.insert(key.to_owned());
            *seen = Some(set);
        }
        false
    }

    fn read_text_until_lt(&mut self) -> Result<String, FormatError> {
        let end = self.rest().find('<').unwrap_or(self.rest().len());
        let raw = &self.rest()[..end];
        let start_offset = self.pos;
        self.pos += end;
        decode_xml_entities(raw, start_offset)
    }
}

fn is_name_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | ':')
}

/// Decode standard XML character entities (`&lt;`, `&gt;`, `&amp;`, `&apos;`, `&quot;`) and
/// numeric character references (`&#123;`, `&#x1F;`).
pub fn decode_xml_entities(raw: &str, base_offset: usize) -> Result<String, FormatError> {
    if !raw.contains('&') {
        return Ok(raw.to_owned());
    }
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(amp_idx) = rest.find('&') {
        out.push_str(&rest[..amp_idx]);
        let after_amp = &rest[amp_idx + 1..];
        let Some(semi_idx) = after_amp.find(';') else {
            return Err(FormatError::XmlParse { offset: base_offset, reason: String::from("unterminated XML entity reference") });
        };
        let entity = &after_amp[..semi_idx];
        if let Some(ch) = lookup_named_xml_entity(entity) {
            out.push(ch);
        } else if entity.starts_with("#x") || entity.starts_with("#X") {
            let code = u32::from_str_radix(&entity[2..], 16)
                .map_err(|_| FormatError::XmlParse { offset: base_offset, reason: format!("invalid hex XML entity `&{entity};`") })?;
            let ch = char::from_u32(code)
                .ok_or_else(|| FormatError::XmlParse { offset: base_offset, reason: format!("invalid Unicode scalar `&{entity};`") })?;
            out.push(ch);
        } else if let Some(digits) = entity.strip_prefix('#') {
            let code = digits
                .parse::<u32>()
                .map_err(|_| FormatError::XmlParse { offset: base_offset, reason: format!("invalid decimal XML entity `&{entity};`") })?;
            let ch = char::from_u32(code)
                .ok_or_else(|| FormatError::XmlParse { offset: base_offset, reason: format!("invalid Unicode scalar `&{entity};`") })?;
            out.push(ch);
        } else {
            return Err(FormatError::XmlParse { offset: base_offset, reason: format!("unknown XML entity `&{entity};`") });
        }
        rest = &after_amp[semi_idx + 1..];
    }
    out.push_str(rest);
    Ok(out)
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
