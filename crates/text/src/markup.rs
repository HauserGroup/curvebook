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
    in_math: bool,
}

impl StyleState {
    fn from_script(base_script: SpanScript) -> Self {
        Self { bold_depth: 0, italic_depth: 0, base_script, script_stack: Vec::new(), in_math: false }
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
        Self { bold_depth: self.bold_depth, italic_depth: self.italic_depth, base_script: script, script_stack: Vec::new(), in_math: self.in_math }
    }

    fn with_bold(&self) -> Self {
        Self { bold_depth: self.bold_depth.saturating_add(1), ..self.clone() }
    }

    fn with_italic(&self) -> Self {
        Self { italic_depth: self.italic_depth.saturating_add(1), ..self.clone() }
    }

    fn with_upright(&self) -> Self {
        Self { bold_depth: 0, italic_depth: 0, base_script: self.current_script(), script_stack: Vec::new(), in_math: self.in_math }
    }

    fn with_roman_operator(&self) -> Self {
        Self { bold_depth: self.bold_depth, italic_depth: 0, base_script: self.current_script(), script_stack: Vec::new(), in_math: self.in_math }
    }

    fn with_math(&self) -> Self {
        Self { in_math: true, ..self.clone() }
    }
}

/// Parse scientific rich-text markup into [`RichText`].
///
/// Supports:
/// - Inline tags: `<b>`..`</b>`, `<i>`..`</i>`, `<sup>`..`</sup>`, `<sub>`..`</sub>` (case-insensitive).
/// - Markdown emphasis: `**bold**`, `*italic*`, `***bold italic***` (preserving unpaired significance stars).
/// - Balanced inline TeX math delimiters (`$...$` / `$$...$$`, preserving currency like `"$100"` or `"Costs $5 to $10"`).
/// - Braced and token super/subscripts: `^{...}`, `^token`, `_{...}`, `_token`.
/// - TeX-subset commands: `\frac{num}{den}`, `\sqrt{expr}`, roman operators (`\log`, `\ln`, `\exp`,
///   `\sin`, `\cos`, `\tan`, `\max`, `\min`, etc.), `\textbf{...}`, `\textit{...}`, `\text{...}`, `\mathrm{...}`,
///   plus LaTeX and HTML named Greek/math escapes (`\alpha`..`\omega`, `\Alpha`..`\Omega`, `\pm`, `\mu`,
///   `\cdot`, `\le`, `\ge`, `\times`, `\approx`, `\infty`, `\neq`, `\Delta`, `\chi`, `&alpha;`, etc.).
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

        if ch == '$'
            && let Some((consumed, next_prev)) = handle_dollar_math(rest, state, spans, depth)
        {
            idx += consumed;
            prev_char = next_prev;
            continue;
        }

        if ch == '*'
            && !state.in_math
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

        if ch == '{'
            && state.in_math
            && let Some((inner, consumed)) = extract_braced(rest)
        {
            let mut inner_state = state.clone();
            parse_segment(inner, &mut inner_state, spans, depth + 1);
            idx += consumed;
            prev_char = Some('}');
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

        if matches!(name, "frac" | "dfrac" | "tfrac")
            && let Some((num, den, args_consumed)) = extract_frac_args(tail)
        {
            return emit_parsed_fraction(num, den, state, spans, depth, 1 + name_len + args_consumed);
        }

        if name == "sqrt"
            && let Some((inner, brace_consumed)) = extract_braced_opt_ws(tail)
        {
            return emit_parsed_sqrt(inner, state, spans, depth, 1 + name_len + brace_consumed);
        }

        if matches!(name, "textbf" | "mathbf" | "textit" | "mathit" | "text" | "mathrm" | "textrm" | "operatorname" | "mbox")
            && let Some((inner, brace_consumed)) = extract_braced_opt_ws(tail)
        {
            let mut inner_state = match name {
                "textbf" | "mathbf" => state.with_bold(),
                "textit" | "mathit" => state.with_italic(),
                _ => state.with_upright(),
            };
            parse_segment(inner, &mut inner_state, spans, depth + 1);
            return Some((1 + name_len + brace_consumed, Some('}')));
        }

        if matches!(name, "left" | "right")
            && let Some((delim, delim_consumed)) = parse_tex_delimiter(tail)
        {
            push_styled(spans, delim, state);
            return Some((1 + name_len + delim_consumed, delim.chars().last()));
        }

        if let Some(op) = lookup_roman_operator(name) {
            let op_state = state.with_roman_operator();
            push_styled(spans, op, &op_state);
            return Some((1 + name_len, op.chars().last()));
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

    if matches!(first, ',' | ';' | ':' | ' ') {
        push_styled(spans, " ", state);
        return Some((1 + first.len_utf8(), Some(' ')));
    }

    if first == '!' {
        return Some((2, None));
    }

    None
}

fn handle_dollar_math(rest: &str, state: &StyleState, spans: &mut Vec<RichSpan>, depth: usize) -> Option<(usize, Option<char>)> {
    let delim_len = if rest.starts_with("$$") { 2 } else { 1 };
    let (inner, consumed) = match_balanced_math_delimiters(rest, delim_len)?;
    let mut math_state = state.with_math();
    parse_segment(inner, &mut math_state, spans, depth + 1);
    Some((consumed, Some('$')))
}

fn match_balanced_math_delimiters(rest: &str, delim_len: usize) -> Option<(&str, usize)> {
    let after_open = rest.get(delim_len..)?;
    if delim_len == 1 && after_open.starts_with('$') {
        return None;
    }

    let bytes = after_open.as_bytes();
    let mut i = 0usize;
    let mut brace_depth = 0usize;

    while i < bytes.len() {
        let ch = after_open[i..].chars().next()?;
        if ch == '\\' {
            i += 1;
            if let Some(next_ch) = after_open[i..].chars().next() {
                i += next_ch.len_utf8();
            }
            continue;
        }
        if ch == '{' {
            brace_depth += 1;
            if brace_depth > MAX_MARKUP_DEPTH {
                return None;
            }
            i += 1;
            continue;
        }
        if ch == '}' {
            if brace_depth == 0 {
                return None;
            }
            brace_depth -= 1;
            i += 1;
            continue;
        }
        if brace_depth == 0 && ((delim_len == 1 && ch == '$') || (delim_len == 2 && after_open[i..].starts_with("$$"))) {
            let inner = &after_open[..i];
            let after_close = &after_open[i + delim_len..];
            if is_math_mode_content(inner, after_close) {
                return Some((inner, delim_len + i + delim_len));
            }
            return None;
        }
        i += ch.len_utf8();
    }

    None
}

fn is_math_mode_content(inner: &str, after_close: &str) -> bool {
    if inner.trim().is_empty() {
        return false;
    }
    if after_close.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        return false;
    }
    if inner.ends_with(char::is_whitespace) && after_close.chars().next().is_some_and(char::is_alphanumeric) {
        return false;
    }
    has_math_indicator(inner) || is_pure_math_identifier(inner)
}

fn has_math_indicator(s: &str) -> bool {
    s.chars().any(is_math_indicator_char)
}

const fn is_math_indicator_char(ch: char) -> bool {
    if matches!(ch, '=' | '\\' | '^' | '_' | '+' | '-' | '/' | '*' | '<' | '>' | '{' | '}') {
        return true;
    }
    let cp = ch as u32;
    matches!(
        cp,
        0x00B0..=0x00B3 | 0x00B5 | 0x00B7 | 0x00B9 | 0x00D7 | 0x00F7 | 0x0370..=0x03FF | 0x2032..=0x2033 | 0x2190..=0x22FF
    )
}

fn is_pure_math_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_ascii_alphabetic() && chars.all(|c| c.is_ascii_alphanumeric() || c == '\'')
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
        let mut inner_state = state.with_script(target_script).with_math();
        parse_segment(inner, &mut inner_state, spans, depth + 1);
        return Some((1 + consumed, Some('}')));
    }

    if first == '\\' {
        let after_bs = after.get(1..)?;
        if after_bs.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
            let script_state = state.with_script(target_script);
            let (consumed, next_prev) = handle_backslash(after, &script_state, spans, depth + 1)?;
            return Some((1 + consumed, next_prev));
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

fn extract_braced_opt_ws(s: &str) -> Option<(&str, usize)> {
    let trimmed = s.trim_start_matches([' ', '\t']);
    let ws_len = s.len() - trimmed.len();
    let (inner, consumed) = extract_braced(trimmed)?;
    Some((inner, ws_len + consumed))
}

fn extract_frac_args(s: &str) -> Option<(&str, &str, usize)> {
    let (num, num_consumed) = extract_braced_opt_ws(s)?;
    let (den, den_consumed) = extract_braced_opt_ws(&s[num_consumed..])?;
    Some((num, den, num_consumed + den_consumed))
}

fn trim_horiz_ws_nonempty(s: &str) -> &str {
    let trimmed = s.trim_matches([' ', '\t']);
    if trimmed.is_empty() { s } else { trimmed }
}

fn spans_plain_text(spans: &[RichSpan]) -> String {
    let mut out = String::new();
    for span in spans {
        out.push_str(&span.text);
    }
    out
}

fn is_enclosed_in_parens(s: &str) -> bool {
    if !(s.starts_with('(') && s.ends_with(')')) {
        return false;
    }
    let mut depth = 0usize;
    for (idx, ch) in s.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return idx + ch.len_utf8() == s.len();
                }
            }
            _ => {}
        }
    }
    false
}

fn is_simple_fraction_char(ch: char) -> bool {
    ch.is_alphanumeric() || matches!(ch, '.' | '\u{2032}' | '\u{2033}')
}

fn needs_fraction_parens(spans: &[RichSpan]) -> bool {
    let plain = spans_plain_text(spans);
    if plain.is_empty() {
        return true;
    }
    if is_enclosed_in_parens(&plain) {
        return false;
    }
    !(plain.chars().all(is_simple_fraction_char) && plain.chars().any(char::is_alphanumeric))
}

fn emit_parsed_fraction(
    num: &str,
    den: &str,
    state: &StyleState,
    spans: &mut Vec<RichSpan>,
    depth: usize,
    total_consumed: usize,
) -> Option<(usize, Option<char>)> {
    let mut math_state = state.with_math();
    let mut num_spans = Vec::new();
    parse_segment(trim_horiz_ws_nonempty(num), &mut math_state, &mut num_spans, depth + 1);
    let mut den_spans = Vec::new();
    parse_segment(trim_horiz_ws_nonempty(den), &mut math_state, &mut den_spans, depth + 1);

    let wrap_num = needs_fraction_parens(&num_spans);
    let wrap_den = needs_fraction_parens(&den_spans);

    if wrap_num {
        push_styled(spans, "(", state);
    }
    for span in num_spans {
        push_rich_span(spans, span);
    }
    if wrap_num {
        push_styled(spans, ")", state);
    }
    push_styled(spans, "/", state);
    if wrap_den {
        push_styled(spans, "(", state);
    }
    for span in den_spans {
        push_rich_span(spans, span);
    }
    if wrap_den {
        push_styled(spans, ")", state);
    }

    let last_ch = spans.last().and_then(|s| s.text.chars().last());
    Some((total_consumed, last_ch))
}

fn emit_parsed_sqrt(
    inner: &str,
    state: &StyleState,
    spans: &mut Vec<RichSpan>,
    depth: usize,
    total_consumed: usize,
) -> Option<(usize, Option<char>)> {
    let mut math_state = state.with_math();
    let mut inner_spans = Vec::new();
    parse_segment(trim_horiz_ws_nonempty(inner), &mut math_state, &mut inner_spans, depth + 1);

    push_styled(spans, "\u{221A}", state);
    let plain = spans_plain_text(&inner_spans);
    if !plain.is_empty() {
        let single_token = inner_spans.len() == 1 && plain.chars().count() == 1 && plain.chars().all(char::is_alphanumeric);
        let wrap = !single_token && !is_enclosed_in_parens(&plain);
        if wrap {
            push_styled(spans, "(", state);
        }
        for span in inner_spans {
            push_rich_span(spans, span);
        }
        if wrap {
            push_styled(spans, ")", state);
        }
    }

    let last_ch = spans.last().and_then(|s| s.text.chars().last());
    Some((total_consumed, last_ch))
}

fn parse_tex_delimiter(s: &str) -> Option<(&'static str, usize)> {
    let trimmed = s.trim_start_matches([' ', '\t']);
    let ws = s.len() - trimmed.len();
    if trimmed.starts_with('(') {
        Some(("(", ws + 1))
    } else if trimmed.starts_with(')') {
        Some((")", ws + 1))
    } else if trimmed.starts_with('[') {
        Some(("[", ws + 1))
    } else if trimmed.starts_with(']') {
        Some(("]", ws + 1))
    } else if trimmed.starts_with('|') {
        Some(("|", ws + 1))
    } else if trimmed.starts_with('.') {
        Some(("", ws + 1))
    } else if trimmed.starts_with("\\{") {
        Some(("{", ws + 2))
    } else if trimmed.starts_with("\\}") {
        Some(("}", ws + 2))
    } else if trimmed.starts_with("\\|") {
        Some(("\u{2016}", ws + 2))
    } else {
        None
    }
}

fn lookup_roman_operator(name: &str) -> Option<&'static str> {
    match name {
        "log" => Some("log"),
        "ln" => Some("ln"),
        "exp" => Some("exp"),
        "sin" => Some("sin"),
        "cos" => Some("cos"),
        "tan" => Some("tan"),
        "sinh" => Some("sinh"),
        "cosh" => Some("cosh"),
        "tanh" => Some("tanh"),
        "arcsin" => Some("arcsin"),
        "arccos" => Some("arccos"),
        "arctan" => Some("arctan"),
        "sec" => Some("sec"),
        "csc" => Some("csc"),
        "cot" => Some("cot"),
        "max" => Some("max"),
        "min" => Some("min"),
        "lim" => Some("lim"),
        "sup" => Some("sup"),
        "inf" => Some("inf"),
        "arg" => Some("arg"),
        "det" => Some("det"),
        "dim" => Some("dim"),
        "gcd" => Some("gcd"),
        "lg" => Some("lg"),
        "erf" => Some("erf"),
        "erfc" => Some("erfc"),
        _ => None,
    }
}

fn push_rich_span(spans: &mut Vec<RichSpan>, span: RichSpan) {
    if span.text.is_empty() {
        return;
    }
    if let Some(last) = spans.last_mut()
        && last.script == span.script
        && last.weight == span.weight
        && last.slope == span.slope
        && last.color == span.color
        && last.font_family == span.font_family
        && (last.scale - span.scale).abs() <= 1e-9
    {
        last.text.push_str(&span.text);
        return;
    }
    spans.push(span);
}

fn push_styled(spans: &mut Vec<RichSpan>, text: &str, state: &StyleState) {
    if text.is_empty() {
        return;
    }
    let script = state.current_script();
    let weight = state.current_weight();
    let slope = state.current_slope();
    let scale = match script {
        SpanScript::Baseline => 1.0,
        SpanScript::Superscript | SpanScript::Subscript => 0.7,
    };
    push_rich_span(spans, RichSpan { text: text.to_owned(), script, weight, slope, color: None, font_family: None, scale });
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
        "quad" => Some("  "),
        "qquad" => Some("    "),
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

// ---------------------------------------------------------------------------
// Structured 2D Equation Segmentation (for stacked fraction vinculum layout)
// ---------------------------------------------------------------------------

/// A segment of a structured 2D mathematical equation supporting stacked fractions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum EquationSegment {
    /// An inline styled rich-text run (variables, operators, scripts, Greek/math symbols).
    Inline(RichText),
    /// A stacked 2D fraction with a horizontal vinculum bar between `numerator` and `denominator`.
    Fraction { numerator: Vec<EquationSegment>, denominator: Vec<EquationSegment> },
}

/// Parse a TeX-subset equation into structured [`EquationSegment`] nodes for 2D layout.
///
/// Unwraps enclosing balanced `$...$` / `$$...$$` math delimiters (preserving currency literals
/// such as `"$100"` or `"Costs $5 to $10"`), splits top-level `\frac{num}{den}` sequences into
/// recursive [`EquationSegment::Fraction`] nodes, and parses inline runs via [`parse_markup`].
/// Empty inline segments are omitted.
#[must_use]
pub fn parse_equation_segments(input: &str) -> Vec<EquationSegment> {
    let mut out = Vec::new();
    let trimmed = input.trim();
    let source = unwrap_outer_math_delimiters(trimmed).unwrap_or(input);
    parse_equation_segments_inner(source, &mut out, 0);
    out
}

fn unwrap_outer_math_delimiters(trimmed: &str) -> Option<&str> {
    if trimmed.starts_with("$$") {
        let (inner, consumed) = match_balanced_math_delimiters(trimmed, 2)?;
        return (consumed == trimmed.len()).then_some(inner);
    }
    if trimmed.starts_with('$') {
        let (inner, consumed) = match_balanced_math_delimiters(trimmed, 1)?;
        return (consumed == trimmed.len()).then_some(inner);
    }
    None
}

fn match_math_for_segments(rest: &str) -> Option<(&str, usize)> {
    let delim_len = if rest.starts_with("$$") { 2 } else { 1 };
    match_balanced_math_delimiters(rest, delim_len)
}

fn parse_equation_segments_inner(input: &str, out: &mut Vec<EquationSegment>, depth: usize) {
    if depth >= MAX_MARKUP_DEPTH {
        push_inline_segment(out, parse_markup(input));
        return;
    }
    let mut idx = 0usize;
    let mut inline_start = 0usize;
    let mut brace_depth = 0usize;

    while idx < input.len() {
        let rest = &input[idx..];
        if brace_depth == 0
            && rest.starts_with('$')
            && let Some((inner, consumed)) = match_math_for_segments(rest)
        {
            push_inline_segment(out, parse_markup(&input[inline_start..idx]));
            parse_equation_segments_inner(inner, out, depth + 1);
            idx += consumed;
            inline_start = idx;
            continue;
        }

        if rest.starts_with('\\') {
            if brace_depth == 0
                && let Some((frac_seg, consumed)) = try_parse_frac_segment(rest, depth)
            {
                push_inline_segment(out, parse_markup(&input[inline_start..idx]));
                out.push(frac_seg);
                idx += consumed;
                inline_start = idx;
                continue;
            }
            idx += step_backslash_in_segment(rest);
            continue;
        }

        let Some(ch) = rest.chars().next() else {
            break;
        };
        if ch == '{' {
            brace_depth = brace_depth.saturating_add(1);
        } else if ch == '}' {
            brace_depth = brace_depth.saturating_sub(1);
        }
        idx += ch.len_utf8();
    }

    if inline_start < input.len() {
        push_inline_segment(out, parse_markup(&input[inline_start..]));
    }
}

fn try_parse_frac_segment(rest: &str, depth: usize) -> Option<(EquationSegment, usize)> {
    let after = rest.get(1..)?;
    let name_len = after.bytes().take_while(u8::is_ascii_alphabetic).count();
    let name = after.get(..name_len)?;
    if !matches!(name, "frac" | "dfrac" | "tfrac") {
        return None;
    }
    let tail = &after[name_len..];
    let (num, den, args_consumed) = extract_frac_args(tail)?;
    let mut numerator = Vec::new();
    parse_equation_segments_inner(trim_horiz_ws_nonempty(num), &mut numerator, depth + 1);
    let mut denominator = Vec::new();
    parse_equation_segments_inner(trim_horiz_ws_nonempty(den), &mut denominator, depth + 1);
    Some((EquationSegment::Fraction { numerator, denominator }, 1 + name_len + args_consumed))
}

fn step_backslash_in_segment(rest: &str) -> usize {
    let Some(after) = rest.get(1..) else {
        return rest.len();
    };
    let name_len = after.bytes().take_while(u8::is_ascii_alphabetic).count();
    if name_len > 0 {
        return 1 + name_len;
    }
    match after.chars().next() {
        Some(ch) => 1 + ch.len_utf8(),
        None => 1,
    }
}

fn push_inline_segment(out: &mut Vec<EquationSegment>, rich: RichText) {
    if rich.is_empty() {
        return;
    }
    if let Some(EquationSegment::Inline(prev)) = out.last_mut() {
        for span in rich.spans {
            push_rich_span(&mut prev.spans, span);
        }
        return;
    }
    out.push(EquationSegment::Inline(rich));
}

// ---------------------------------------------------------------------------
// Bidirectional (BiDi) & RTL Text Shaping (UAX #9 subset + bracket mirroring)
// ---------------------------------------------------------------------------

/// Base or resolved run text direction for bidirectional scientific text layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum TextDirection {
    #[default]
    Ltr,
    Rtl,
}

/// A contiguous visually ordered run produced by [`shape_bidi_runs`].
///
/// `start_byte..end_byte` records the corresponding logical UTF-8 byte slice in the original
/// input string, while `text` contains the visually ordered (and for [`TextDirection::Rtl`],
/// bracket-mirrored) UTF-8 characters ready for left-to-right glyph emission.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BidiRun {
    pub text: String,
    pub direction: TextDirection,
    pub start_byte: usize,
    pub end_byte: usize,
}

/// Return `true` when BCP-47 locale `code` denotes a right-to-left language
/// (`"ar"`, `"he"`, `"fa"`, `"ur"`), including region or script subtags such as `"ar-SA"`.
#[must_use]
pub fn is_rtl_locale(code: &str) -> bool {
    let mut parts = code.trim().split(['-', '_']);
    let Some(lang) = parts.next() else {
        return false;
    };
    let is_rtl_lang =
        lang.eq_ignore_ascii_case("ar") || lang.eq_ignore_ascii_case("he") || lang.eq_ignore_ascii_case("fa") || lang.eq_ignore_ascii_case("ur");
    if !is_rtl_lang {
        return false;
    }
    !parts.any(|sub| sub.eq_ignore_ascii_case("latn"))
}

/// Return `true` if `text` contains any Hebrew or Arabic script character.
///
/// Covers Hebrew (`U+0590..=U+05FF`, `U+FB1D..=U+FB4F`) and Arabic (`U+0600..=U+06FF`,
/// Supplement `U+0750..=U+077F`, Extended `U+0870..=U+08FF`, Presentation `U+FB50..=U+FDFF` /
/// `U+FE70..=U+FEFF`).
#[must_use]
pub fn contains_rtl_script(text: &str) -> bool {
    text.chars().any(is_rtl_script_char)
}

const fn is_rtl_script_char(ch: char) -> bool {
    matches!(
        ch as u32,
        0x0590..=0x05FF
            | 0x0600..=0x06FF
            | 0x0750..=0x077F
            | 0x0870..=0x08FF
            | 0xFB1D..=0xFB4F
            | 0xFB50..=0xFDFF
            | 0xFE70..=0xFEFF
    )
}

/// Shape `text` into visually ordered (left-to-right) [`BidiRun`] segments under `base_dir`.
///
/// Handles mixed Arabic/Hebrew text with embedded ASCII/numbers/math identifiers (`IC50`, `12.4`,
/// `α + β`), multi-line paragraph breaks, combining marks, and Unicode bracket mirroring
/// (`()`, `[]`, `{}`, `<>`, etc.) inside RTL runs.
#[must_use]
pub fn shape_bidi_runs(text: &str, base_dir: TextDirection) -> Vec<BidiRun> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut runs = Vec::new();
    let mut line_start = 0usize;

    while line_start < text.len() {
        let rest = &text[line_start..];
        let (line_rel_end, sep_rel_end) = find_paragraph_break(rest);
        let line_end = line_start + line_rel_end;
        let sep_end = line_start + sep_rel_end;

        if line_end > line_start {
            shape_bidi_line(&text[line_start..line_end], line_start, base_dir, &mut runs);
        }
        if sep_end > line_end {
            push_merged_run(
                &mut runs,
                BidiRun { text: text[line_end..sep_end].to_owned(), direction: TextDirection::Ltr, start_byte: line_end, end_byte: sep_end },
            );
        }
        line_start = sep_end;
    }

    runs
}

/// Return `text` reordered into visual left-to-right display order under `base_dir`.
#[must_use]
pub fn reorder_visual_bidi(text: &str, base_dir: TextDirection) -> String {
    let runs = shape_bidi_runs(text, base_dir);
    let mut out = String::with_capacity(text.len());
    for run in runs {
        out.push_str(&run.text);
    }
    out
}

fn find_paragraph_break(rest: &str) -> (usize, usize) {
    for (idx, ch) in rest.char_indices() {
        if ch == '\r' {
            let next_len = if rest[idx + 1..].starts_with('\n') { 2 } else { 1 };
            return (idx, idx + next_len);
        }
        if matches!(ch, '\n' | '\u{0085}' | '\u{2029}') {
            return (idx, idx + ch.len_utf8());
        }
    }
    (rest.len(), rest.len())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BidiClass {
    L,
    R,
    Al,
    En,
    Es,
    Et,
    An,
    Cs,
    Nsm,
    Ws,
    S,
    On,
}

#[derive(Debug, Clone, Copy)]
struct BidiChar {
    ch: char,
    byte_start: usize,
    byte_end: usize,
    orig_class: BidiClass,
    resolved_class: BidiClass,
    level: u8,
}

#[derive(Debug, Clone)]
struct VisualCluster {
    text: String,
    direction: TextDirection,
    level: u8,
    start_byte: usize,
    end_byte: usize,
}

const fn needs_bidi_resolution(ch: char) -> bool {
    is_rtl_script_char(ch)
        || matches!(
            ch as u32,
            0x061C | 0x0700..=0x074F | 0x0780..=0x085F | 0x200E..=0x200F | 0x202A..=0x202E | 0x2066..=0x2069
        )
}

fn shape_bidi_line(line: &str, base_offset: usize, base_dir: TextDirection, out: &mut Vec<BidiRun>) {
    if base_dir == TextDirection::Ltr && !line.chars().any(needs_bidi_resolution) {
        push_merged_run(
            out,
            BidiRun { text: line.to_owned(), direction: TextDirection::Ltr, start_byte: base_offset, end_byte: base_offset + line.len() },
        );
        return;
    }

    let para_level: u8 = match base_dir {
        TextDirection::Ltr => 0,
        TextDirection::Rtl => 1,
    };
    let embed_dir = if para_level.is_multiple_of(2) { BidiClass::L } else { BidiClass::R };

    let mut chars: Vec<BidiChar> = line
        .char_indices()
        .map(|(rel_idx, ch)| {
            let cls = classify_bidi_char(ch);
            BidiChar {
                ch,
                byte_start: base_offset + rel_idx,
                byte_end: base_offset + rel_idx + ch.len_utf8(),
                orig_class: cls,
                resolved_class: cls,
                level: para_level,
            }
        })
        .collect();
    if chars.is_empty() {
        return;
    }

    resolve_weak_types(&mut chars, embed_dir);
    resolve_brackets_and_neutrals(&mut chars, embed_dir);
    resolve_implicit_levels(&mut chars, para_level);
    reset_trailing_whitespace_levels(&mut chars, para_level);

    let mut clusters = build_visual_clusters(&chars);
    reorder_clusters_l2(&mut clusters);

    for cluster in clusters {
        push_merged_run(
            out,
            BidiRun { text: cluster.text, direction: cluster.direction, start_byte: cluster.start_byte, end_byte: cluster.end_byte },
        );
    }
}

fn resolve_weak_types(chars: &mut [BidiChar], sos: BidiClass) {
    // W1: NSM takes previous character's type (or sos at start).
    let mut prev = sos;
    for item in chars.iter_mut() {
        if item.resolved_class == BidiClass::Nsm {
            item.resolved_class = prev;
        }
        prev = item.resolved_class;
    }

    // W2: EN preceded by AL becomes AN.
    let mut last_strong = sos;
    for item in chars.iter_mut() {
        match item.resolved_class {
            BidiClass::L | BidiClass::R | BidiClass::Al => last_strong = item.resolved_class,
            BidiClass::En if last_strong == BidiClass::Al => item.resolved_class = BidiClass::An,
            _ => {}
        }
    }

    // W3: AL -> R.
    for item in chars.iter_mut() {
        if item.resolved_class == BidiClass::Al {
            item.resolved_class = BidiClass::R;
        }
    }

    // W4: Single ES between ENs -> EN; single CS between two ENs -> EN or two ANs -> AN.
    for i in 1..chars.len().saturating_sub(1) {
        let prev_cls = chars[i - 1].resolved_class;
        let next_cls = chars[i + 1].resolved_class;
        match chars[i].resolved_class {
            BidiClass::Es if prev_cls == BidiClass::En && next_cls == BidiClass::En => {
                chars[i].resolved_class = BidiClass::En;
            }
            BidiClass::Cs if prev_cls == BidiClass::En && next_cls == BidiClass::En => {
                chars[i].resolved_class = BidiClass::En;
            }
            BidiClass::Cs if prev_cls == BidiClass::An && next_cls == BidiClass::An => {
                chars[i].resolved_class = BidiClass::An;
            }
            _ => {}
        }
    }

    // W5: Sequences of ET adjacent to EN -> EN.
    let mut adjacent_en = false;
    for item in chars.iter_mut() {
        match item.resolved_class {
            BidiClass::En => adjacent_en = true,
            BidiClass::Et if adjacent_en => item.resolved_class = BidiClass::En,
            _ => adjacent_en = false,
        }
    }
    adjacent_en = false;
    for item in chars.iter_mut().rev() {
        match item.resolved_class {
            BidiClass::En => adjacent_en = true,
            BidiClass::Et if adjacent_en => item.resolved_class = BidiClass::En,
            _ => adjacent_en = false,
        }
    }

    // W6: Remaining ES/ET/CS -> ON.
    for item in chars.iter_mut() {
        if matches!(item.resolved_class, BidiClass::Es | BidiClass::Et | BidiClass::Cs) {
            item.resolved_class = BidiClass::On;
        }
    }

    // W7: EN preceded by strong L -> L.
    last_strong = sos;
    for item in chars.iter_mut() {
        match item.resolved_class {
            BidiClass::L | BidiClass::R => last_strong = item.resolved_class,
            BidiClass::En if last_strong == BidiClass::L => item.resolved_class = BidiClass::L,
            _ => {}
        }
    }
}

fn resolve_brackets_and_neutrals(chars: &mut [BidiChar], embed_dir: BidiClass) {
    // N0: Paired bracket resolution (UAX #9 BD16).
    let opp_dir = if embed_dir == BidiClass::L { BidiClass::R } else { BidiClass::L };
    let pairs = find_bracket_pairs(chars);
    for (open_idx, close_idx) in pairs {
        let mut has_embed = false;
        let mut has_opp = false;
        for item in chars.iter().take(close_idx).skip(open_idx + 1) {
            if item.resolved_class == embed_dir {
                has_embed = true;
                break;
            }
            if item.resolved_class == opp_dir {
                has_opp = true;
            }
        }
        let chosen = if has_embed {
            Some(embed_dir)
        } else if has_opp {
            let preceding = chars[..open_idx]
                .iter()
                .rev()
                .find_map(|c| matches!(c.resolved_class, BidiClass::L | BidiClass::R).then_some(c.resolved_class))
                .unwrap_or(embed_dir);
            Some(if preceding == opp_dir { opp_dir } else { embed_dir })
        } else {
            None
        };
        if let Some(dir) = chosen {
            chars[open_idx].resolved_class = dir;
            chars[close_idx].resolved_class = dir;
            apply_bracket_nsm(chars, open_idx + 1, dir);
            apply_bracket_nsm(chars, close_idx + 1, dir);
        }
    }

    // N1 & N2: Neutrals flanked by matching effective strong types take that type, else embed_dir.
    let n = chars.len();
    let mut i = 0usize;
    while i < n {
        if !is_neutral_class(chars[i].resolved_class) {
            i += 1;
            continue;
        }
        let start = i;
        while i < n && is_neutral_class(chars[i].resolved_class) {
            i += 1;
        }
        let prev_dir = if start == 0 { embed_dir } else { neutral_effective_strong(chars[start - 1].resolved_class, embed_dir) };
        let next_dir = if i >= n { embed_dir } else { neutral_effective_strong(chars[i].resolved_class, embed_dir) };
        let fill = if prev_dir == next_dir { prev_dir } else { embed_dir };
        for item in &mut chars[start..i] {
            item.resolved_class = fill;
        }
    }
}

fn find_bracket_pairs(chars: &[BidiChar]) -> Vec<(usize, usize)> {
    const MAX_STACK: usize = 63;
    let mut stack: Vec<(usize, char)> = Vec::new();
    let mut pairs = Vec::new();
    for (idx, item) in chars.iter().enumerate() {
        if item.resolved_class != BidiClass::On {
            continue;
        }
        if let Some(closing) = paired_bracket_closing(item.ch) {
            if stack.len() >= MAX_STACK {
                break;
            }
            stack.push((idx, closing));
        } else if is_paired_closing_bracket(item.ch)
            && let Some(pos) = stack.iter().rposition(|&(_, expected)| expected == item.ch || canonical_bracket_eq(expected, item.ch))
        {
            let (open_idx, _) = stack[pos];
            stack.truncate(pos);
            pairs.push((open_idx, idx));
        }
    }
    pairs.sort_unstable_by_key(|&(open_idx, _)| open_idx);
    pairs
}

fn apply_bracket_nsm(chars: &mut [BidiChar], mut idx: usize, dir: BidiClass) {
    while let Some(item) = chars.get_mut(idx) {
        if item.orig_class != BidiClass::Nsm {
            break;
        }
        item.resolved_class = dir;
        idx += 1;
    }
}

const fn is_neutral_class(cls: BidiClass) -> bool {
    matches!(cls, BidiClass::On | BidiClass::Ws | BidiClass::S)
}

const fn neutral_effective_strong(cls: BidiClass, fallback: BidiClass) -> BidiClass {
    match cls {
        BidiClass::L => BidiClass::L,
        BidiClass::R | BidiClass::En | BidiClass::An => BidiClass::R,
        _ => fallback,
    }
}

fn resolve_implicit_levels(chars: &mut [BidiChar], embed_level: u8) {
    let even = embed_level.is_multiple_of(2);
    for item in chars {
        item.level = if even {
            match item.resolved_class {
                BidiClass::L => embed_level,
                BidiClass::R => embed_level + 1,
                BidiClass::An | BidiClass::En => embed_level + 2,
                _ => embed_level,
            }
        } else {
            match item.resolved_class {
                BidiClass::R => embed_level,
                BidiClass::L | BidiClass::An | BidiClass::En => embed_level + 1,
                _ => embed_level,
            }
        };
    }
}

fn reset_trailing_whitespace_levels(chars: &mut [BidiChar], para_level: u8) {
    for item in chars.iter_mut().rev() {
        if matches!(item.orig_class, BidiClass::Ws | BidiClass::S) {
            item.level = para_level;
        } else {
            break;
        }
    }
}

fn build_visual_clusters(chars: &[BidiChar]) -> Vec<VisualCluster> {
    let mut clusters: Vec<VisualCluster> = Vec::with_capacity(chars.len());
    for item in chars {
        let is_rtl = !item.level.is_multiple_of(2);
        let ch = if is_rtl { mirror_bidi_char(item.ch) } else { item.ch };
        let direction = if is_rtl { TextDirection::Rtl } else { TextDirection::Ltr };
        if item.orig_class == BidiClass::Nsm
            && let Some(prev) = clusters.last_mut()
            && prev.level == item.level
            && prev.end_byte == item.byte_start
        {
            prev.text.push(ch);
            prev.end_byte = item.byte_end;
            continue;
        }
        let mut text = String::with_capacity(ch.len_utf8());
        text.push(ch);
        clusters.push(VisualCluster { text, direction, level: item.level, start_byte: item.byte_start, end_byte: item.byte_end });
    }
    clusters
}

fn reorder_clusters_l2(clusters: &mut [VisualCluster]) {
    let max_level = clusters.iter().map(|c| c.level).max().unwrap_or(0);
    for lvl in (1..=max_level).rev() {
        let mut i = 0usize;
        while i < clusters.len() {
            if clusters[i].level < lvl {
                i += 1;
                continue;
            }
            let start = i;
            while i < clusters.len() && clusters[i].level >= lvl {
                i += 1;
            }
            clusters[start..i].reverse();
        }
    }
}

fn push_merged_run(runs: &mut Vec<BidiRun>, next: BidiRun) {
    if next.text.is_empty() {
        return;
    }
    if let Some(prev) = runs.last_mut()
        && prev.direction == next.direction
    {
        match prev.direction {
            TextDirection::Ltr if prev.end_byte == next.start_byte => {
                prev.text.push_str(&next.text);
                prev.end_byte = next.end_byte;
                return;
            }
            TextDirection::Rtl if next.end_byte == prev.start_byte => {
                prev.text.push_str(&next.text);
                prev.start_byte = next.start_byte;
                return;
            }
            _ => {}
        }
    }
    runs.push(next);
}

const fn paired_bracket_closing(ch: char) -> Option<char> {
    match ch {
        '(' => Some(')'),
        '[' => Some(']'),
        '{' => Some('}'),
        '\u{27E8}' => Some('\u{27E9}'),
        '\u{3008}' => Some('\u{3009}'),
        '\u{300A}' => Some('\u{300B}'),
        '\u{300C}' => Some('\u{300D}'),
        '\u{300E}' => Some('\u{300F}'),
        '\u{3010}' => Some('\u{3011}'),
        '\u{FF08}' => Some('\u{FF09}'),
        '\u{FF3B}' => Some('\u{FF3D}'),
        '\u{FF5B}' => Some('\u{FF5D}'),
        _ => None,
    }
}

const fn is_paired_closing_bracket(ch: char) -> bool {
    matches!(ch, ')' | ']' | '}' | '\u{27E9}' | '\u{3009}' | '\u{300B}' | '\u{300D}' | '\u{300F}' | '\u{3011}' | '\u{FF09}' | '\u{FF3D}' | '\u{FF5D}')
}

const fn canonical_bracket_eq(a: char, b: char) -> bool {
    matches!((a, b), ('\u{27E9}', '\u{3009}') | ('\u{3009}', '\u{27E9}'))
}

const fn mirror_bidi_char(ch: char) -> char {
    match ch {
        '(' => ')',
        ')' => '(',
        '[' => ']',
        ']' => '[',
        '{' => '}',
        '}' => '{',
        '<' => '>',
        '>' => '<',
        '\u{00AB}' => '\u{00BB}',
        '\u{00BB}' => '\u{00AB}',
        '\u{2039}' => '\u{203A}',
        '\u{203A}' => '\u{2039}',
        '\u{2264}' => '\u{2265}',
        '\u{2265}' => '\u{2264}',
        '\u{226A}' => '\u{226B}',
        '\u{226B}' => '\u{226A}',
        '\u{2282}' => '\u{2283}',
        '\u{2283}' => '\u{2282}',
        '\u{2286}' => '\u{2287}',
        '\u{2287}' => '\u{2286}',
        '\u{27E8}' => '\u{27E9}',
        '\u{27E9}' => '\u{27E8}',
        '\u{3008}' => '\u{3009}',
        '\u{3009}' => '\u{3008}',
        '\u{300A}' => '\u{300B}',
        '\u{300B}' => '\u{300A}',
        '\u{300C}' => '\u{300D}',
        '\u{300D}' => '\u{300C}',
        '\u{300E}' => '\u{300F}',
        '\u{300F}' => '\u{300E}',
        '\u{3010}' => '\u{3011}',
        '\u{3011}' => '\u{3010}',
        '\u{FF08}' => '\u{FF09}',
        '\u{FF09}' => '\u{FF08}',
        '\u{FF1C}' => '\u{FF1E}',
        '\u{FF1E}' => '\u{FF1C}',
        '\u{FF3B}' => '\u{FF3D}',
        '\u{FF3D}' => '\u{FF3B}',
        '\u{FF5B}' => '\u{FF5D}',
        '\u{FF5D}' => '\u{FF5B}',
        _ => ch,
    }
}

const fn classify_bidi_char(ch: char) -> BidiClass {
    let cp = ch as u32;
    if matches!(cp, 0x0009 | 0x000B | 0x001F) {
        return BidiClass::S;
    }
    if matches!(cp, 0x0020 | 0x000C | 0x1680 | 0x2000..=0x200A | 0x2028 | 0x205F | 0x3000) {
        return BidiClass::Ws;
    }
    if is_bidi_nsm(cp) {
        return BidiClass::Nsm;
    }
    if matches!(
        cp,
        0x0030..=0x0039 | 0x00B2 | 0x00B3 | 0x00B9 | 0x06F0..=0x06F9 | 0x2070 | 0x2074..=0x2079 | 0x2080..=0x2089
    ) {
        return BidiClass::En;
    }
    if matches!(cp, 0x0600..=0x0605 | 0x0660..=0x0669 | 0x066B..=0x066C | 0x06DD | 0x070F | 0x08E2) {
        return BidiClass::An;
    }
    if matches!(cp, 0x002B | 0x002D | 0x207A..=0x207B | 0x208A..=0x208B | 0x2212) {
        return BidiClass::Es;
    }
    if matches!(cp, 0x0023..=0x0025 | 0x00A2..=0x00A5 | 0x00B0 | 0x00B1 | 0x066A | 0x2030..=0x2034 | 0x20A0..=0x20CF) {
        return BidiClass::Et;
    }
    if matches!(cp, 0x002C | 0x002E | 0x002F | 0x003A | 0x00A0 | 0x060C | 0x202F | 0x2044) {
        return BidiClass::Cs;
    }
    if matches!(cp, 0x0590..=0x05FF | 0x0700..=0x074F | 0x0780..=0x085F | 0x200F | 0xFB1D..=0xFB4F) {
        return BidiClass::R;
    }
    if matches!(
        cp,
        0x0600..=0x06FF | 0x0750..=0x077F | 0x0870..=0x08FF | 0xFB50..=0xFDFF | 0xFE70..=0xFEFF
    ) {
        return BidiClass::Al;
    }
    if is_bidi_other_neutral(cp) {
        return BidiClass::On;
    }
    BidiClass::L
}

const fn is_bidi_nsm(cp: u32) -> bool {
    matches!(
        cp,
        0x0300..=0x036F
            | 0x0483..=0x0489
            | 0x0591..=0x05BD
            | 0x05BF
            | 0x05C1..=0x05C2
            | 0x05C4..=0x05C5
            | 0x05C7
            | 0x0610..=0x061A
            | 0x064B..=0x065F
            | 0x0670
            | 0x06D6..=0x06DC
            | 0x06DF..=0x06E4
            | 0x06E7..=0x06E8
            | 0x06EA..=0x06ED
            | 0x0711
            | 0x0730..=0x074A
            | 0x07A6..=0x07B0
            | 0x07EB..=0x07F3
            | 0x0816..=0x082D
            | 0x0859..=0x085B
            | 0x08D3..=0x08E1
            | 0x08E3..=0x08FF
            | 0x20D0..=0x20FF
            | 0xFB1E
            | 0xFE00..=0xFE0F
            | 0xFE20..=0xFE2F
    )
}

const fn is_bidi_other_neutral(cp: u32) -> bool {
    matches!(
        cp,
        0x0021..=0x0022
            | 0x0026..=0x002A
            | 0x003B..=0x0040
            | 0x005B..=0x0060
            | 0x007B..=0x007E
            | 0x00A1
            | 0x00A6..=0x00A9
            | 0x00AB..=0x00AF
            | 0x00B4
            | 0x00B6..=0x00B8
            | 0x00BB..=0x00BF
            | 0x00D7
            | 0x00F7
            | 0x2010..=0x2027
            | 0x2035..=0x205E
            | 0x2190..=0x2BFF
            | 0x3001..=0x3020
            | 0xFF01..=0xFF0F
            | 0xFF1A..=0xFF20
            | 0xFF3B..=0xFF40
            | 0xFF5B..=0xFF65
    )
}

#[cfg(test)]
mod tests {
    use super::{EquationSegment, RichText, SpanScript, SpanSlope, SpanWeight, parse_equation_segments, parse_markup};

    fn concat_segment_plain(segments: &[EquationSegment]) -> String {
        let mut out = String::new();
        for seg in segments {
            match seg {
                EquationSegment::Inline(rich) => out.push_str(&rich.plain_text()),
                EquationSegment::Fraction { numerator, denominator } => {
                    out.push('(');
                    out.push_str(&concat_segment_plain(numerator));
                    out.push_str(")/(");
                    out.push_str(&concat_segment_plain(denominator));
                    out.push(')');
                }
            }
        }
        out
    }

    #[test]
    fn parse_markup_strips_math_dollars_and_preserves_currency_and_unpaired_dollars() {
        assert_eq!(parse_markup("$100").plain_text(), "$100");
        assert_eq!(parse_markup("Costs $5 to $10").plain_text(), "Costs $5 to $10");
        assert_eq!(parse_markup("Costs $5 - $10 or $20").plain_text(), "Costs $5 - $10 or $20");
        assert_eq!(parse_markup("$50 + $100 = $150").plain_text(), "$50 + $100 = $150");
        assert_eq!(parse_markup("Costs $5 + tax, or $x = 1$").plain_text(), "Costs $5 + tax, or x = 1");
        assert_eq!(parse_markup(r"\$5 + \$10").plain_text(), "$5 + $10");
        assert_eq!(parse_markup("$$").plain_text(), "$$");
        assert_eq!(parse_markup("$x$ + $y$").plain_text(), "x + y");
        assert_eq!(parse_markup(r"$\alpha + \beta = \gamma$").plain_text(), "α + β = γ");
        assert_eq!(parse_markup(r"$$Y = \frac{1}{2}$$").plain_text(), "Y = 1/2");
    }

    #[test]
    fn parse_markup_renders_dose_response_4pl_and_michaelis_menten_and_decay() {
        let eq_4pl = r"$Y = \text{Bottom} + \frac{\text{Top} - \text{Bottom}}{1 + 10^{(\log\text{EC}_{50} - X) \cdot \text{HillSlope}}}$";
        let rich_4pl = parse_markup(eq_4pl);
        assert_eq!(rich_4pl.plain_text(), "Y = Bottom + (Top - Bottom)/(1 + 10(logEC50 - X) · HillSlope)");
        assert!(rich_4pl.spans.iter().any(|s| s.text == "50" && s.script == SpanScript::Subscript));
        assert!(rich_4pl.spans.iter().any(|s| s.text.contains("HillSlope") && s.script == SpanScript::Superscript));
        assert!(rich_4pl.spans.iter().all(|s| !s.text.contains('$') && !s.text.contains('\\')));

        let mm = r"Y = \frac{V_{\max} \cdot X}{K_m + X}";
        let rich_mm = parse_markup(mm);
        assert_eq!(rich_mm.plain_text(), "Y = (Vmax · X)/(Km + X)");
        assert!(rich_mm.spans.iter().any(|s| s.text == "max" && s.script == SpanScript::Subscript && s.slope == SpanSlope::Upright));
        assert!(rich_mm.spans.iter().any(|s| s.text == "m" && s.script == SpanScript::Subscript));

        let decay = r"Y = (Y_0 - \text{Plateau}) \cdot \exp(-K \cdot X) + \text{Plateau}";
        let rich_decay = parse_markup(decay);
        assert_eq!(rich_decay.plain_text(), "Y = (Y0 - Plateau) · exp(-K · X) + Plateau");
        assert!(rich_decay.spans.iter().any(|s| s.text == "0" && s.script == SpanScript::Subscript));
        assert!(rich_decay.spans.iter().any(|s| s.text.contains("exp") && s.slope == SpanSlope::Upright));
    }

    #[test]
    fn parse_markup_handles_sqrt_roman_operators_math_symbols_and_nested_braces() {
        let sq_multi = parse_markup(r"\sqrt{a^2 + b^2}");
        assert_eq!(sq_multi.plain_text(), "√(a2 + b2)");
        assert!(sq_multi.spans.iter().any(|s| s.text == "2" && s.script == SpanScript::Superscript));

        assert_eq!(parse_markup(r"\sqrt{x}").plain_text(), "√x");
        assert_eq!(parse_markup(r"\sqrt{(a + b)}").plain_text(), "√(a + b)");
        assert_eq!(parse_markup(r"\sqrt").plain_text(), "√");

        let ops = parse_markup(r"<i>\log x + \ln y + \exp z + \sin\theta + \cos\phi + \tan\psi + \max(a) + \min(b)</i>");
        assert_eq!(ops.plain_text(), "log x + ln y + exp z + sinθ + cosφ + tanψ + max(a) + min(b)");
        for op in ["log", "ln", "exp", "sin", "cos", "tan", "max", "min"] {
            assert!(ops.spans.iter().any(|s| s.text.contains(op) && s.slope == SpanSlope::Upright), "missing upright op {op}");
        }

        let bold_max = parse_markup(r"\textbf{V_{\max}}");
        assert_eq!(bold_max.plain_text(), "Vmax");
        assert!(bold_max.spans.iter().any(|s| s.text == "max" && s.weight == SpanWeight::Bold && s.script == SpanScript::Subscript));

        let nested_frac = parse_markup(r"\frac{1 + \frac{a}{b}}{c + {d + e}}");
        assert_eq!(nested_frac.plain_text(), "(1 + a/b)/(c + d + e)");

        let simple_frac = parse_markup(r"\frac{a}{b}");
        assert_eq!(simple_frac.plain_text(), "a/b");

        let symbols = parse_markup(r"$a \cdot b \times c \pm d \le e \leq f \ge g \geq h \infty \approx i \ne j \neq k$");
        assert_eq!(symbols.plain_text(), "a · b × c ± d ≤ e ≤ f ≥ g ≥ h ∞ ≈ i ≠ j ≠ k");

        let left_right = parse_markup(r"$\left(\frac{a + b}{c}\right)$");
        assert_eq!(left_right.plain_text(), "((a + b)/c)");
    }

    #[test]
    fn parse_equation_segments_builds_stacked_ast_for_models() {
        let eq_4pl = r"$Y = \text{Bottom} + \frac{\text{Top} - \text{Bottom}}{1 + 10^{(\log\text{EC}_{50} - X) \cdot \text{HillSlope}}}$";
        let segs_4pl = parse_equation_segments(eq_4pl);
        assert_eq!(segs_4pl.len(), 2);
        assert!(matches!(&segs_4pl[0], EquationSegment::Inline(rich) if rich.plain_text() == "Y = Bottom + "));
        let EquationSegment::Fraction { numerator, denominator } = &segs_4pl[1] else {
            panic!("expected fraction segment, got {:?}", segs_4pl[1]);
        };
        assert_eq!(numerator, &[EquationSegment::Inline(RichText::plain("Top - Bottom"))]);
        assert_eq!(denominator.len(), 1);
        let EquationSegment::Inline(rich) = &denominator[0] else {
            panic!("expected inline denominator, got {:?}", denominator[0]);
        };
        assert_eq!(rich.plain_text(), "1 + 10(logEC50 - X) · HillSlope");
        assert!(rich.spans.iter().any(|s| s.text == "50" && s.script == SpanScript::Subscript));

        let mm = r"Y = \frac{V_{\max} \cdot X}{K_m + X}";
        let segs_mm = parse_equation_segments(mm);
        assert_eq!(segs_mm.len(), 2);
        assert_eq!(concat_segment_plain(&segs_mm), "Y = (Vmax · X)/(Km + X)");

        let nested = parse_equation_segments(r"$Y = \frac{1}{1 + \frac{a}{b}}$");
        assert_eq!(nested.len(), 2);
        let EquationSegment::Fraction { numerator, denominator } = &nested[1] else {
            panic!("expected outer fraction, got {:?}", nested[1]);
        };
        assert_eq!(numerator, &[EquationSegment::Inline(RichText::plain("1"))]);
        assert_eq!(denominator.len(), 2);
        assert!(matches!(&denominator[0], EquationSegment::Inline(r) if r.plain_text() == "1 + "));
        assert!(matches!(&denominator[1], EquationSegment::Fraction { .. }));
    }

    #[test]
    fn parse_equation_segments_handles_braced_scripts_currency_and_empty_inputs() {
        // Top-level frac inside superscript stays inline rather than splitting braced scripts.
        let exp_frac = parse_equation_segments(r"$Y = e^{\frac{-x}{2}}$");
        assert_eq!(exp_frac.len(), 1);
        assert!(matches!(&exp_frac[0], EquationSegment::Inline(r) if r.plain_text() == "Y = e(-x)/2"));

        // Currency and empty inputs.
        assert_eq!(parse_equation_segments("$100"), vec![EquationSegment::Inline(RichText::plain("$100"))]);
        assert_eq!(parse_equation_segments("Costs $5 to $10"), vec![EquationSegment::Inline(RichText::plain("Costs $5 to $10"))]);
        assert!(parse_equation_segments("").is_empty());
        assert!(parse_equation_segments("<b></b>").is_empty());

        // Omit empty inline segments when fraction is standalone.
        let standalone = parse_equation_segments(r"\frac{a}{b}");
        assert_eq!(standalone.len(), 1);
        assert!(matches!(standalone[0], EquationSegment::Fraction { .. }));
    }
}
