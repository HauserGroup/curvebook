//! Scientific rich-text markup parsing and template-hook expansion for figure labels.

use lumencraft_palette::Rgba8;
use serde::{Deserialize, Serialize};

const MAX_MARKUP_DEPTH: usize = 64;

/// Baseline placement of a rich-text span.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum SpanScript {
    #[default]
    Baseline,
    Superscript,
    Subscript,
}

/// Font weight for a rich-text span.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum SpanWeight {
    #[default]
    Regular,
    Bold,
}

/// Font slope for a rich-text span.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum SpanSlope {
    #[default]
    Upright,
    Italic,
}

/// A styled segment inside a [`RichText`] paragraph or label.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RichSpan {
    pub text: String,
    pub script: SpanScript,
    pub weight: SpanWeight,
    pub slope: SpanSlope,
    pub color: Option<Rgba8>,
    pub font_family: Option<String>,
    pub scale: f64,
}

impl RichSpan {
    /// Create an unstyled baseline span with default scale `1.0`.
    #[must_use]
    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            script: SpanScript::Baseline,
            weight: SpanWeight::Regular,
            slope: SpanSlope::Upright,
            color: None,
            font_family: None,
            scale: 1.0,
        }
    }
}

impl Default for RichSpan {
    fn default() -> Self {
        Self::plain(String::new())
    }
}

/// Ordered sequence of styled spans composing a single scientific label or caption.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct RichText {
    pub spans: Vec<RichSpan>,
}

impl RichText {
    /// Construct a [`RichText`] from pre-built spans.
    #[must_use]
    pub fn new(spans: Vec<RichSpan>) -> Self {
        Self { spans }
    }

    /// Construct a single plain baseline span when `text` is non-empty.
    #[must_use]
    pub fn plain(text: impl Into<String>) -> Self {
        let s = text.into();
        if s.is_empty() { Self { spans: Vec::new() } } else { Self { spans: vec![RichSpan::plain(s)] } }
    }

    /// Concatenate unstyled plain characters across all spans.
    #[must_use]
    pub fn plain_text(&self) -> String {
        let mut out = String::new();
        for span in &self.spans {
            out.push_str(&span.text);
        }
        out
    }

    /// True if all spans have empty text.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.spans.iter().all(|s| s.text.is_empty())
    }
}

#[derive(Debug, Clone)]
struct StyleState {
    bold_depth: usize,
    italic_depth: usize,
    base_script: SpanScript,
    script_stack: Vec<SpanScript>,
}

impl StyleState {
    fn from_script(base_script: SpanScript) -> Self {
        Self { bold_depth: 0, italic_depth: 0, base_script, script_stack: Vec::new() }
    }

    fn current_weight(&self) -> SpanWeight {
        if self.bold_depth > 0 { SpanWeight::Bold } else { SpanWeight::Regular }
    }

    fn current_slope(&self) -> SpanSlope {
        if self.italic_depth > 0 { SpanSlope::Italic } else { SpanSlope::Upright }
    }

    fn current_script(&self) -> SpanScript {
        self.script_stack.last().copied().unwrap_or(self.base_script)
    }

    fn with_script(&self, script: SpanScript) -> Self {
        Self { bold_depth: self.bold_depth, italic_depth: self.italic_depth, base_script: script, script_stack: Vec::new() }
    }

    fn with_bold(&self) -> Self {
        Self { bold_depth: self.bold_depth.saturating_add(1), ..self.clone() }
    }

    fn with_italic(&self) -> Self {
        Self { italic_depth: self.italic_depth.saturating_add(1), ..self.clone() }
    }

    fn with_upright(&self) -> Self {
        Self { bold_depth: 0, italic_depth: 0, base_script: self.current_script(), script_stack: Vec::new() }
    }
}

/// Parse scientific rich-text markup into [`RichText`].
///
/// Supports:
/// - Inline tags: `<b>`..`</b>`, `<i>`..`</i>`, `<sup>`..`</sup>`, `<sub>`..`</sub>` (case-insensitive).
/// - Markdown emphasis: `**bold**`, `*italic*`, `***bold italic***` (preserving unpaired significance stars).
/// - Braced and token super/subscripts: `^{...}`, `^token`, `_{...}`, `_token`.
/// - LaTeX and HTML named Greek/math escapes (`\alpha`..`\omega`, `\Alpha`..`\Omega`, `\pm`, `\mu`,
///   `\le`, `\ge`, `\times`, `\approx`, `\infty`, `\Delta`, `\chi`, `&alpha;`, etc.), plus `\textbf{...}`,
///   `\textit{...}`, `\text{...}`, `\mathrm{...}`.
#[must_use]
pub fn parse_markup(input: &str) -> RichText {
    let mut spans = Vec::new();
    let mut state = StyleState::from_script(SpanScript::Baseline);
    parse_segment(input, &mut state, &mut spans, 0);
    RichText::new(spans)
}

fn parse_segment(input: &str, state: &mut StyleState, spans: &mut Vec<RichSpan>, depth: usize) {
    if depth >= MAX_MARKUP_DEPTH {
        push_styled(spans, input, state);
        return;
    }
    let mut idx = 0usize;
    let mut prev_char: Option<char> = None;

    while idx < input.len() {
        let rest = &input[idx..];

        if let Some((tag, consumed)) = parse_html_tag(rest) {
            apply_html_tag(tag, state);
            idx += consumed;
            prev_char = Some('>');
            continue;
        }

        let Some(ch) = rest.chars().next() else {
            break;
        };

        if ch == '\\'
            && let Some((consumed, next_prev)) = handle_backslash(rest, state, spans, depth)
        {
            idx += consumed;
            prev_char = next_prev;
            continue;
        }

        if ch == '&'
            && let Some((decoded, consumed)) = parse_html_entity(rest)
        {
            push_styled(spans, &decoded, state);
            idx += consumed;
            prev_char = decoded.chars().last();
            continue;
        }

        if ch == '*'
            && let Some((consumed, next_prev)) = handle_asterisk_run(rest, prev_char, state, spans, depth)
        {
            idx += consumed;
            prev_char = next_prev;
            continue;
        }

        if (ch == '^' || ch == '_')
            && let Some((consumed, next_prev)) = handle_script_marker(rest, ch, state, spans, depth)
        {
            idx += consumed;
            prev_char = next_prev;
            continue;
        }

        let ch_len = ch.len_utf8();
        push_styled(spans, &rest[..ch_len], state);
        idx += ch_len;
        prev_char = Some(ch);
    }
}

#[derive(Debug, Clone, Copy)]
enum HtmlTag {
    BoldOpen,
    BoldClose,
    ItalicOpen,
    ItalicClose,
    SupOpen,
    SupClose,
    SubOpen,
    SubClose,
}

fn parse_html_tag(rest: &str) -> Option<(HtmlTag, usize)> {
    if !rest.starts_with('<') {
        return None;
    }
    let close = rest.find('>')?;
    if close == 0 || close > 8 {
        return None;
    }
    let inner = &rest[1..close];
    let tag = if inner.eq_ignore_ascii_case("b") {
        HtmlTag::BoldOpen
    } else if inner.eq_ignore_ascii_case("/b") {
        HtmlTag::BoldClose
    } else if inner.eq_ignore_ascii_case("i") {
        HtmlTag::ItalicOpen
    } else if inner.eq_ignore_ascii_case("/i") {
        HtmlTag::ItalicClose
    } else if inner.eq_ignore_ascii_case("sup") {
        HtmlTag::SupOpen
    } else if inner.eq_ignore_ascii_case("/sup") {
        HtmlTag::SupClose
    } else if inner.eq_ignore_ascii_case("sub") {
        HtmlTag::SubOpen
    } else if inner.eq_ignore_ascii_case("/sub") {
        HtmlTag::SubClose
    } else {
        return None;
    };
    Some((tag, close + 1))
}

fn apply_html_tag(tag: HtmlTag, state: &mut StyleState) {
    match tag {
        HtmlTag::BoldOpen => state.bold_depth = state.bold_depth.saturating_add(1),
        HtmlTag::BoldClose => state.bold_depth = state.bold_depth.saturating_sub(1),
        HtmlTag::ItalicOpen => state.italic_depth = state.italic_depth.saturating_add(1),
        HtmlTag::ItalicClose => state.italic_depth = state.italic_depth.saturating_sub(1),
        HtmlTag::SupOpen => {
            if state.script_stack.len() < MAX_MARKUP_DEPTH {
                state.script_stack.push(SpanScript::Superscript);
            }
        }
        HtmlTag::SubOpen => {
            if state.script_stack.len() < MAX_MARKUP_DEPTH {
                state.script_stack.push(SpanScript::Subscript);
            }
        }
        HtmlTag::SupClose => pop_script_tag(state, SpanScript::Superscript),
        HtmlTag::SubClose => pop_script_tag(state, SpanScript::Subscript),
    }
}

fn pop_script_tag(state: &mut StyleState, target: SpanScript) {
    if state.script_stack.last().copied() == Some(target) {
        let _ = state.script_stack.pop();
    } else if let Some(pos) = state.script_stack.iter().rposition(|&s| s == target) {
        let _ = state.script_stack.remove(pos);
    }
}

fn handle_backslash(rest: &str, state: &StyleState, spans: &mut Vec<RichSpan>, depth: usize) -> Option<(usize, Option<char>)> {
    let after = rest.get(1..)?;
    let first = after.chars().next()?;

    if first.is_ascii_alphabetic() {
        let name_len = after.bytes().take_while(u8::is_ascii_alphabetic).count();
        let name = &after[..name_len];
        let tail = &after[name_len..];

        if tail.starts_with('{')
            && matches!(name, "textbf" | "mathbf" | "textit" | "mathit" | "text" | "mathrm")
            && let Some((inner, brace_consumed)) = extract_braced(tail)
        {
            let mut inner_state = match name {
                "textbf" | "mathbf" => state.with_bold(),
                "textit" | "mathit" => state.with_italic(),
                _ => state.with_upright(),
            };
            parse_segment(inner, &mut inner_state, spans, depth + 1);
            return Some((1 + name_len + brace_consumed, Some('}')));
        }

        if let Some(sym) = lookup_latex_escape(name) {
            push_styled(spans, sym, state);
            return Some((1 + name_len, sym.chars().last()));
        }

        return None;
    }

    if matches!(first, '\\' | '{' | '}' | '^' | '_' | '*' | '<' | '>' | '&' | '%' | '$' | '#') {
        let mut buf = [0u8; 4];
        let s = first.encode_utf8(&mut buf);
        push_styled(spans, s, state);
        return Some((1 + first.len_utf8(), Some(first)));
    }

    None
}

fn handle_asterisk_run(
    rest: &str,
    prev_char: Option<char>,
    state: &StyleState,
    spans: &mut Vec<RichSpan>,
    depth: usize,
) -> Option<(usize, Option<char>)> {
    let run_len = rest.bytes().take_while(|&b| b == b'*').count();
    let after_open = &rest[run_len..];
    let next_char = after_open.chars().next()?;

    if run_len >= 4 || next_char.is_whitespace() || next_char == '*' {
        return None;
    }

    if run_len == 3 {
        let close_offset = find_closing_star_run(after_open, 3, true)?;
        let inner = &after_open[..close_offset];
        let mut inner_state = state.with_bold().with_italic();
        parse_segment(inner, &mut inner_state, spans, depth + 1);
        return Some((3 + close_offset + 3, Some('*')));
    }

    if run_len == 2 {
        if prev_char.is_some_and(|c| c.is_ascii_digit()) && next_char.is_ascii_digit() {
            return None;
        }
        let close_offset = find_closing_star_run(after_open, 2, false)?;
        let inner = &after_open[..close_offset];
        let mut inner_state = state.with_bold();
        parse_segment(inner, &mut inner_state, spans, depth + 1);
        return Some((2 + close_offset + 2, Some('*')));
    }

    // Single '*' italic delimiter: avoid splitting numeric multiplication like `1*10^3 + 2*10^4`.
    if prev_char.is_some_and(|c| c.is_ascii_digit()) && next_char.is_ascii_digit() {
        return None;
    }
    let close_offset = find_closing_star_run(after_open, 1, false)?;
    let inner = &after_open[..close_offset];
    let mut inner_state = state.with_italic();
    parse_segment(inner, &mut inner_state, spans, depth + 1);
    Some((1 + close_offset + 1, Some('*')))
}

fn find_closing_star_run(s: &str, needed: usize, exact: bool) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            i += 1;
            if let Some(ch) = s[i..].chars().next() {
                i += ch.len_utf8();
            }
            continue;
        }
        if bytes[i] != b'*' {
            let Some(ch) = s[i..].chars().next() else {
                break;
            };
            i += ch.len_utf8();
            continue;
        }
        let run_len = bytes[i..].iter().take_while(|&&b| b == b'*').count();
        let prev_non_space = i > 0 && s[..i].chars().next_back().is_some_and(|c| !c.is_whitespace() && c != '*');
        let len_matches = if exact { run_len == needed } else { run_len == needed || (needed == 2 && run_len == 3) };
        if prev_non_space && len_matches {
            let offset = if needed == 2 && run_len == 3 { i + 1 } else { i };
            return Some(offset);
        }
        i += run_len;
    }
    None
}

fn handle_script_marker(rest: &str, marker: char, state: &StyleState, spans: &mut Vec<RichSpan>, depth: usize) -> Option<(usize, Option<char>)> {
    let target_script = if marker == '^' { SpanScript::Superscript } else { SpanScript::Subscript };
    let after = &rest[1..];
    let first = after.chars().next()?;

    if first == '{' {
        let (inner, consumed) = extract_braced(after)?;
        let mut inner_state = state.with_script(target_script);
        parse_segment(inner, &mut inner_state, spans, depth + 1);
        return Some((1 + consumed, Some('}')));
    }

    if first == '\\' {
        let after_bs = after.get(1..)?;
        let name_len = after_bs.bytes().take_while(u8::is_ascii_alphabetic).count();
        if name_len > 0 {
            let name = &after_bs[..name_len];
            if let Some(sym) = lookup_latex_escape(name) {
                let script_state = state.with_script(target_script);
                push_styled(spans, sym, &script_state);
                return Some((2 + name_len, sym.chars().last()));
            }
        }
        return None;
    }

    let token_len = scan_script_token(after)?;
    let token = &after[..token_len];
    let script_state = state.with_script(target_script);
    push_styled(spans, token, &script_state);
    Some((1 + token_len, token.chars().last()))
}

fn scan_script_token(after: &str) -> Option<usize> {
    let mut chars = after.char_indices();
    let (_, first) = chars.next()?;

    let mut end = if first == '-' || first == '\u{2212}' {
        let (_, second) = chars.next()?;
        if !second.is_alphanumeric() {
            return None;
        }
        first.len_utf8() + second.len_utf8()
    } else if first.is_alphanumeric() {
        first.len_utf8()
    } else {
        return None;
    };

    for (_, ch) in after[end..].char_indices() {
        if ch.is_alphanumeric() {
            end += ch.len_utf8();
        } else {
            break;
        }
    }
    Some(end)
}

fn extract_braced(s: &str) -> Option<(&str, usize)> {
    if !s.starts_with('{') {
        return None;
    }
    let mut depth = 0usize;
    let mut escaped = false;
    for (idx, ch) in s.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == '{' {
            depth += 1;
            if depth > MAX_MARKUP_DEPTH {
                return None;
            }
        } else if ch == '}' {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return Some((&s[1..idx], idx + 1));
            }
        }
    }
    None
}

fn push_styled(spans: &mut Vec<RichSpan>, text: &str, state: &StyleState) {
    if text.is_empty() {
        return;
    }
    let script = state.current_script();
    let weight = state.current_weight();
    let slope = state.current_slope();
    if let Some(last) = spans.last_mut()
        && last.script == script
        && last.weight == weight
        && last.slope == slope
        && last.color.is_none()
        && last.font_family.is_none()
    {
        last.text.push_str(text);
        return;
    }
    let scale = match script {
        SpanScript::Baseline => 1.0,
        SpanScript::Superscript | SpanScript::Subscript => 0.7,
    };
    spans.push(RichSpan { text: text.to_owned(), script, weight, slope, color: None, font_family: None, scale });
}

fn lookup_latex_escape(name: &str) -> Option<&'static str> {
    if let Some(greek) = lookup_greek_escape(name) {
        return Some(greek);
    }
    lookup_math_escape(name)
}

fn lookup_greek_escape(name: &str) -> Option<&'static str> {
    match name {
        "alpha" => Some("\u{03B1}"),
        "beta" => Some("\u{03B2}"),
        "gamma" => Some("\u{03B3}"),
        "delta" => Some("\u{03B4}"),
        "epsilon" => Some("\u{03B5}"),
        "varepsilon" => Some("\u{03F5}"),
        "zeta" => Some("\u{03B6}"),
        "eta" => Some("\u{03B7}"),
        "theta" => Some("\u{03B8}"),
        "vartheta" => Some("\u{03D1}"),
        "iota" => Some("\u{03B9}"),
        "kappa" => Some("\u{03BA}"),
        "varkappa" => Some("\u{03F0}"),
        "lambda" => Some("\u{03BB}"),
        "mu" | "micro" => Some("\u{03BC}"),
        "nu" => Some("\u{03BD}"),
        "xi" => Some("\u{03BE}"),
        "omicron" => Some("\u{03BF}"),
        "pi" => Some("\u{03C0}"),
        "varpi" => Some("\u{03D6}"),
        "rho" => Some("\u{03C1}"),
        "varrho" => Some("\u{03F1}"),
        "varsigma" => Some("\u{03C2}"),
        "sigma" => Some("\u{03C3}"),
        "tau" => Some("\u{03C4}"),
        "upsilon" => Some("\u{03C5}"),
        "phi" => Some("\u{03C6}"),
        "varphi" => Some("\u{03D5}"),
        "chi" => Some("\u{03C7}"),
        "psi" => Some("\u{03C8}"),
        "omega" => Some("\u{03C9}"),
        "Alpha" => Some("\u{0391}"),
        "Beta" => Some("\u{0392}"),
        "Gamma" => Some("\u{0393}"),
        "Delta" => Some("\u{0394}"),
        "Epsilon" => Some("\u{0395}"),
        "Zeta" => Some("\u{0396}"),
        "Eta" => Some("\u{0397}"),
        "Theta" => Some("\u{0398}"),
        "Iota" => Some("\u{0399}"),
        "Kappa" => Some("\u{039A}"),
        "Lambda" => Some("\u{039B}"),
        "Mu" => Some("\u{039C}"),
        "Nu" => Some("\u{039D}"),
        "Xi" => Some("\u{039E}"),
        "Omicron" => Some("\u{039F}"),
        "Pi" => Some("\u{03A0}"),
        "Rho" => Some("\u{03A1}"),
        "Sigma" => Some("\u{03A3}"),
        "Tau" => Some("\u{03A4}"),
        "Upsilon" => Some("\u{03A5}"),
        "Phi" => Some("\u{03A6}"),
        "Chi" => Some("\u{03A7}"),
        "Psi" => Some("\u{03A8}"),
        "Omega" | "ohm" => Some("\u{03A9}"),
        _ => None,
    }
}

fn lookup_math_escape(name: &str) -> Option<&'static str> {
    match name {
        "pm" => Some("\u{00B1}"),
        "mp" => Some("\u{2213}"),
        "times" => Some("\u{00D7}"),
        "div" => Some("\u{00F7}"),
        "cdot" => Some("\u{00B7}"),
        "bullet" => Some("\u{2022}"),
        "le" | "leq" => Some("\u{2264}"),
        "ge" | "geq" => Some("\u{2265}"),
        "ne" | "neq" => Some("\u{2260}"),
        "approx" => Some("\u{2248}"),
        "equiv" => Some("\u{2261}"),
        "sim" => Some("\u{223C}"),
        "propto" => Some("\u{221D}"),
        "ll" => Some("\u{226A}"),
        "gg" => Some("\u{226B}"),
        "infty" => Some("\u{221E}"),
        "minus" => Some("\u{2212}"),
        "degree" | "deg" | "textdegree" => Some("\u{00B0}"),
        "circ" => Some("\u{2218}"),
        "prime" => Some("\u{2032}"),
        "dprime" => Some("\u{2033}"),
        "perthousand" | "permil" => Some("\u{2030}"),
        "angstrom" | "AA" => Some("\u{00C5}"),
        "partial" => Some("\u{2202}"),
        "nabla" => Some("\u{2207}"),
        "sum" => Some("\u{2211}"),
        "prod" => Some("\u{220F}"),
        "int" => Some("\u{222B}"),
        "sqrt" => Some("\u{221A}"),
        "to" | "rightarrow" => Some("\u{2192}"),
        "leftarrow" => Some("\u{2190}"),
        "leftrightarrow" => Some("\u{2194}"),
        "uparrow" => Some("\u{2191}"),
        "downarrow" => Some("\u{2193}"),
        "therefore" => Some("\u{2234}"),
        "forall" => Some("\u{2200}"),
        "exists" => Some("\u{2203}"),
        "in" => Some("\u{2208}"),
        "notin" => Some("\u{2209}"),
        "subset" => Some("\u{2282}"),
        "subseteq" => Some("\u{2286}"),
        "cup" => Some("\u{222A}"),
        "cap" => Some("\u{2229}"),
        "emptyset" => Some("\u{2205}"),
        "dag" | "dagger" => Some("\u{2020}"),
        "ddag" | "ddagger" => Some("\u{2021}"),
        "sect" | "S" => Some("\u{00A7}"),
        "copyright" => Some("\u{00A9}"),
        "registered" => Some("\u{00AE}"),
        "trademark" => Some("\u{2122}"),
        "ldots" | "dots" => Some("\u{2026}"),
        "ndash" => Some("\u{2013}"),
        "mdash" => Some("\u{2014}"),
        "percent" => Some("%"),
        "space" => Some(" "),
        _ => None,
    }
}

fn parse_html_entity(rest: &str) -> Option<(String, usize)> {
    let after = rest.strip_prefix('&')?;
    let semi = after.find(';')?;
    if semi == 0 || semi > 16 {
        return None;
    }
    let name = &after[..semi];
    if let Some(hex) = name.strip_prefix("#x").or_else(|| name.strip_prefix("#X")) {
        let code = u32::from_str_radix(hex, 16).ok()?;
        let ch = valid_entity_char(code)?;
        return Some((ch.to_string(), 1 + semi + 1));
    }
    if let Some(dec) = name.strip_prefix('#') {
        let code = dec.parse::<u32>().ok()?;
        let ch = valid_entity_char(code)?;
        return Some((ch.to_string(), 1 + semi + 1));
    }
    let mapped = lookup_html_entity(name)?;
    Some((mapped.to_owned(), 1 + semi + 1))
}

fn valid_entity_char(code: u32) -> Option<char> {
    if code == 0 {
        return None;
    }
    char::from_u32(code)
}

fn lookup_html_entity(name: &str) -> Option<&'static str> {
    if let Some(greek) = lookup_greek_escape(name) {
        return Some(greek);
    }
    match name {
        "pm" | "plusmn" => Some("\u{00B1}"),
        "times" => Some("\u{00D7}"),
        "divide" => Some("\u{00F7}"),
        "middot" => Some("\u{00B7}"),
        "bull" => Some("\u{2022}"),
        "le" => Some("\u{2264}"),
        "ge" => Some("\u{2265}"),
        "ne" => Some("\u{2260}"),
        "asymp" | "approx" => Some("\u{2248}"),
        "equiv" => Some("\u{2261}"),
        "sim" => Some("\u{223C}"),
        "prop" => Some("\u{221D}"),
        "infin" => Some("\u{221E}"),
        "minus" => Some("\u{2212}"),
        "deg" => Some("\u{00B0}"),
        "prime" => Some("\u{2032}"),
        "Prime" => Some("\u{2033}"),
        "permil" => Some("\u{2030}"),
        "angst" => Some("\u{00C5}"),
        "part" => Some("\u{2202}"),
        "nabla" => Some("\u{2207}"),
        "sum" => Some("\u{2211}"),
        "prod" => Some("\u{220F}"),
        "int" => Some("\u{222B}"),
        "radic" => Some("\u{221A}"),
        "rarr" => Some("\u{2192}"),
        "larr" => Some("\u{2190}"),
        "harr" => Some("\u{2194}"),
        "uarr" => Some("\u{2191}"),
        "darr" => Some("\u{2193}"),
        "forall" => Some("\u{2200}"),
        "exist" => Some("\u{2203}"),
        "isin" => Some("\u{2208}"),
        "notin" => Some("\u{2209}"),
        "sub" => Some("\u{2282}"),
        "sube" => Some("\u{2286}"),
        "cup" => Some("\u{222A}"),
        "cap" => Some("\u{2229}"),
        "empty" => Some("\u{2205}"),
        "dagger" => Some("\u{2020}"),
        "Dagger" => Some("\u{2021}"),
        "sect" => Some("\u{00A7}"),
        "copy" => Some("\u{00A9}"),
        "reg" => Some("\u{00AE}"),
        "trade" => Some("\u{2122}"),
        "hellip" => Some("\u{2026}"),
        "ndash" => Some("\u{2013}"),
        "mdash" => Some("\u{2014}"),
        "lt" => Some("<"),
        "gt" => Some(">"),
        "amp" => Some("&"),
        "quot" => Some("\""),
        "apos" => Some("'"),
        "nbsp" => Some("\u{00A0}"),
        _ => None,
    }
}

/// Expand `{{hook_name}}` placeholders using caller-supplied hook resolution.
///
/// Unresolved tokens preserve their literal `{{hook_name}}` placeholder so template authors
/// receive an explicit non-destructive visual cue. Resolution is single-pass so cyclic or
/// nested `{{...}}` values returned by `resolver` never trigger recursive re-expansion.
#[must_use]
pub fn expand_template_hooks<F>(template: &str, mut resolver: F) -> String
where
    F: FnMut(&str) -> Option<String>,
{
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after_open = &rest[start + 2..];
        if let Some(end) = after_open.find("}}") {
            let token = after_open[..end].trim();
            if !token.is_empty()
                && let Some(value) = resolver(token)
            {
                out.push_str(&value);
            } else {
                out.push_str("{{");
                out.push_str(&after_open[..end]);
                out.push_str("}}");
            }
            rest = &after_open[end + 2..];
        } else {
            out.push_str("{{");
            rest = after_open;
        }
    }
    out.push_str(rest);
    out
}
