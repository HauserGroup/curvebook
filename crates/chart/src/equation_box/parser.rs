//! Structured TeX-subset math AST and recursive-descent parser for live model equation boxes.
//!
//! Parses nested fractions, superscripts/subscripts, radicals, scaled delimiters, Greek
//! identifiers, and roman math operators following the math-list grammar of Knuth, D. E. (1984),
//! *The TeXbook*, Addison-Wesley, Chapter 17 (pp. 140–145).

use curvebook_text::{
    SpanScript, SpanSlope, SpanWeight, TexDelimiter, extract_braced, extract_braced_opt_ws, lookup_latex_escape, lookup_roman_operator,
    match_balanced_math_delimiters, parse_tex_delimiter_token, unwrap_outer_math_delimiters,
};

use super::MAX_TEX_DEPTH;

// ---------------------------------------------------------------------------
// Math AST & TeX Parser
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FractionKind {
    Auto,
    Display,
    Text,
    Binomial,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DelimKind {
    Paren,
    Bracket,
    Brace,
    Pipe,
    DoublePipe,
    Null,
}

#[derive(Debug, Clone)]
pub(super) enum MathNode {
    Text { text: String, weight: SpanWeight, slope: SpanSlope, is_single_char: bool },
    Space { width_em: f64 },
    ThinSpace,
    Kern { width_em: f64 },
    Fraction { numerator: Vec<MathNode>, denominator: Vec<MathNode>, kind: FractionKind },
    Radical { degree: Option<Vec<MathNode>>, radicand: Vec<MathNode> },
    Delimited { left: DelimKind, inner: Vec<MathNode>, right: DelimKind, explicit: bool },
    Group(Vec<MathNode>),
    Scripted { nucleus: Box<MathNode>, sup: Option<Vec<MathNode>>, sub: Option<Vec<MathNode>> },
}

pub(super) enum ParsedStep {
    Atom { node: MathNode, is_roman_op: bool },
    Raw(MathNode),
}

pub(super) fn parse_math_nodes(input: &str) -> Vec<MathNode> {
    let trimmed = input.trim();
    let source = unwrap_outer_math_delimiters(trimmed).unwrap_or(input);
    let mut nodes = parse_math_slice(source, false, 0);
    trim_edge_spaces(&mut nodes);
    nodes
}

pub(super) fn parse_math_slice(input: &str, text_mode: bool, depth: usize) -> Vec<MathNode> {
    if depth >= MAX_TEX_DEPTH {
        if input.is_empty() {
            return Vec::new();
        }
        return vec![MathNode::Text { text: input.to_owned(), weight: SpanWeight::Regular, slope: SpanSlope::Upright, is_single_char: false }];
    }

    let mut nodes = Vec::new();
    let mut pos = 0usize;
    while pos < input.len() {
        let prev_pos = pos;
        let Some(rest) = input.get(pos..) else {
            break;
        };
        let Some(ch) = rest.chars().next() else {
            break;
        };
        if ch.is_whitespace() {
            while let Some(c) = input.get(pos..).and_then(|s| s.chars().next())
                && c.is_whitespace()
            {
                pos += c.len_utf8();
            }
            if !matches!(nodes.last(), Some(MathNode::Space { .. })) {
                nodes.push(MathNode::Space { width_em: 0.278 });
            }
            continue;
        }
        if let Some(step) = parse_next_step(input, &mut pos, text_mode, depth) {
            match step {
                ParsedStep::Raw(node) => nodes.push(node),
                ParsedStep::Atom { node, is_roman_op } => {
                    let attached = attach_scripts_if_any(node, input, &mut pos, text_mode, depth);
                    nodes.push(attached);
                    if is_roman_op && needs_operator_thin_space(input.get(pos..).unwrap_or("")) {
                        nodes.push(MathNode::ThinSpace);
                    }
                }
            }
        }
        if pos <= prev_pos {
            // Invariant guard: every loop iteration must strictly advance `pos` so malformed tokens never hang.
            pos = prev_pos + ch.len_utf8();
        }
    }
    nodes
}

pub(super) fn parse_next_step(input: &str, pos: &mut usize, text_mode: bool, depth: usize) -> Option<ParsedStep> {
    let rest = input.get(*pos..)?;
    let ch = rest.chars().next()?;

    if ch == '<'
        && let Some((node, consumed)) = try_parse_html_step(rest, text_mode, depth)
    {
        *pos += consumed;
        return Some(ParsedStep::Atom { node, is_roman_op: false });
    }
    if ch == '\\' {
        return Some(parse_backslash_step(input, pos, depth));
    }
    if ch == '$'
        && let Some((inner, consumed)) = match_balanced_math_delimiters(rest, if rest.starts_with("$$") { 2 } else { 1 })
    {
        *pos += consumed;
        let mut inner_nodes = parse_math_slice(inner, false, depth + 1);
        trim_edge_spaces(&mut inner_nodes);
        return Some(ParsedStep::Atom { node: MathNode::Group(inner_nodes), is_roman_op: false });
    }
    if ch == '{'
        && let Some((inner, consumed)) = extract_braced(rest)
    {
        *pos += consumed;
        let mut inner_nodes = parse_math_slice(inner, text_mode, depth + 1);
        trim_edge_spaces(&mut inner_nodes);
        return Some(ParsedStep::Atom { node: MathNode::Group(inner_nodes), is_roman_op: false });
    }
    if !text_mode && matches!(ch, '(' | '[' | '|') {
        let delim = match ch {
            '(' => DelimKind::Paren,
            '[' => DelimKind::Bracket,
            _ => DelimKind::Pipe,
        };
        if let Some((inner_nodes, consumed)) = try_parse_matched_delim(rest, ch, depth) {
            *pos += consumed;
            return Some(ParsedStep::Atom {
                node: MathNode::Delimited { left: delim, inner: inner_nodes, right: delim, explicit: false },
                is_roman_op: false,
            });
        }
    }
    if ch == '^' || ch == '_' {
        // Standalone script marker without a preceding nucleus: attach to an empty nucleus
        // only when a valid script argument follows; otherwise consume the orphan marker literally.
        let start_pos = *pos;
        let empty = MathNode::Group(Vec::new());
        let attached = attach_scripts_if_any(empty, input, pos, text_mode, depth);
        if *pos > start_pos {
            return Some(ParsedStep::Raw(attached));
        }
    }
    Some(parse_literal_atom(input, pos, text_mode))
}

fn parse_literal_atom(input: &str, pos: &mut usize, text_mode: bool) -> ParsedStep {
    let rest = input.get(*pos..).unwrap_or("");
    let Some(ch) = rest.chars().next() else {
        return ParsedStep::Raw(MathNode::Group(Vec::new()));
    };

    if ch.is_ascii_digit() {
        let mut end = ch.len_utf8();
        while let Some(nxt) = rest.get(end..).and_then(|s| s.chars().next()) {
            if nxt.is_ascii_digit() || (nxt == '.' && rest.get(end + 1..).and_then(|s| s.chars().next()).is_some_and(|d| d.is_ascii_digit())) {
                end += nxt.len_utf8();
            } else {
                break;
            }
        }
        let slice = rest.get(..end).unwrap_or("").to_owned();
        *pos += end;
        return ParsedStep::Atom {
            node: MathNode::Text { text: slice, weight: SpanWeight::Regular, slope: SpanSlope::Upright, is_single_char: true },
            is_roman_op: false,
        };
    }

    if text_mode && (ch.is_ascii_alphanumeric() || matches!(ch, '-' | '.' | '_' | ',' | ':' | ';')) {
        let mut end = ch.len_utf8();
        while let Some(nxt) = rest.get(end..).and_then(|s| s.chars().next()) {
            if nxt.is_ascii_alphanumeric() || matches!(nxt, '-' | '.' | '_') {
                end += nxt.len_utf8();
            } else {
                break;
            }
        }
        let slice = rest.get(..end).unwrap_or("").to_owned();
        *pos += end;
        return ParsedStep::Atom {
            node: MathNode::Text { text: slice, weight: SpanWeight::Regular, slope: SpanSlope::Upright, is_single_char: false },
            is_roman_op: false,
        };
    }

    *pos += ch.len_utf8();
    let slope = if !text_mode && ch.is_ascii_alphabetic() { SpanSlope::Italic } else { SpanSlope::Upright };
    let mut buf = [0u8; 4];
    let text = ch.encode_utf8(&mut buf).to_owned();
    ParsedStep::Atom { node: MathNode::Text { text, weight: SpanWeight::Regular, slope, is_single_char: true }, is_roman_op: false }
}

fn parse_backslash_step(input: &str, pos: &mut usize, depth: usize) -> ParsedStep {
    let rest = input.get(*pos..).unwrap_or("");
    let Some(after) = rest.get(1..) else {
        *pos = input.len();
        return ParsedStep::Raw(MathNode::Group(Vec::new()));
    };
    let Some(first) = after.chars().next() else {
        *pos = input.len();
        return ParsedStep::Raw(MathNode::Group(Vec::new()));
    };

    if first.is_ascii_alphabetic() {
        let name_len = after.bytes().take_while(u8::is_ascii_alphabetic).count();
        let name = after.get(..name_len).unwrap_or("");
        let tail = after.get(name_len..).unwrap_or("");
        if let Some((step, tail_consumed)) = parse_named_command(name, tail, depth) {
            *pos += 1 + name_len + tail_consumed;
            return step;
        }
        *pos += 1 + name_len;
        return ParsedStep::Atom {
            node: MathNode::Text { text: name.to_owned(), weight: SpanWeight::Regular, slope: SpanSlope::Upright, is_single_char: false },
            is_roman_op: false,
        };
    }

    *pos += 1 + first.len_utf8();
    match first {
        ',' => ParsedStep::Raw(MathNode::Kern { width_em: 0.167 }),
        ':' => ParsedStep::Raw(MathNode::Kern { width_em: 0.222 }),
        ';' => ParsedStep::Raw(MathNode::Kern { width_em: 0.278 }),
        ' ' => ParsedStep::Raw(MathNode::Space { width_em: 0.278 }),
        '!' => ParsedStep::Raw(MathNode::Kern { width_em: 0.0 }),
        '|' => ParsedStep::Atom {
            node: MathNode::Text { text: "\u{2016}".to_owned(), weight: SpanWeight::Regular, slope: SpanSlope::Upright, is_single_char: true },
            is_roman_op: false,
        },
        _ => {
            let mut buf = [0u8; 4];
            ParsedStep::Atom {
                node: MathNode::Text {
                    text: first.encode_utf8(&mut buf).to_owned(),
                    weight: SpanWeight::Regular,
                    slope: SpanSlope::Upright,
                    is_single_char: true,
                },
                is_roman_op: false,
            }
        }
    }
}

fn parse_named_command(name: &str, tail: &str, depth: usize) -> Option<(ParsedStep, usize)> {
    if matches!(name, "frac" | "dfrac" | "tfrac" | "binom")
        && let Some((num_raw, den_raw, consumed)) = extract_two_math_args(tail)
    {
        let kind = match name {
            "dfrac" => FractionKind::Display,
            "tfrac" => FractionKind::Text,
            "binom" => FractionKind::Binomial,
            _ => FractionKind::Auto,
        };
        let mut numerator = parse_math_slice(num_raw, false, depth + 1);
        let mut denominator = parse_math_slice(den_raw, false, depth + 1);
        trim_edge_spaces(&mut numerator);
        trim_edge_spaces(&mut denominator);
        return Some((ParsedStep::Atom { node: MathNode::Fraction { numerator, denominator, kind }, is_roman_op: false }, consumed));
    }

    if name == "sqrt"
        && let Some((deg_raw, rad_raw, consumed)) = extract_sqrt_args(tail)
    {
        let degree = deg_raw.map(|d| {
            let mut parsed = parse_math_slice(d, false, depth + 1);
            trim_edge_spaces(&mut parsed);
            parsed
        });
        let mut radicand = parse_math_slice(rad_raw, false, depth + 1);
        trim_edge_spaces(&mut radicand);
        return Some((ParsedStep::Atom { node: MathNode::Radical { degree, radicand }, is_roman_op: false }, consumed));
    }

    if name == "left"
        && let Some((left_delim, inner_slice, right_delim, consumed)) = extract_left_right_pair(tail)
    {
        let mut inner = parse_math_slice(inner_slice, false, depth + 1);
        trim_edge_spaces(&mut inner);
        return Some((
            ParsedStep::Atom { node: MathNode::Delimited { left: left_delim, inner, right: right_delim, explicit: true }, is_roman_op: false },
            consumed,
        ));
    }

    if matches!(name, "textbf" | "mathbf" | "textit" | "mathit" | "text" | "mathrm" | "textrm" | "operatorname" | "mbox")
        && let Some((inner, consumed)) = extract_braced_opt_ws(tail)
    {
        let inner_text_mode = !matches!(name, "textit" | "mathit");
        let mut inner_nodes = parse_math_slice(inner, inner_text_mode, depth + 1);
        let override_weight = matches!(name, "textbf" | "mathbf").then_some(SpanWeight::Bold);
        let override_slope = if matches!(name, "textit" | "mathit") { Some(SpanSlope::Italic) } else { Some(SpanSlope::Upright) };
        apply_style_override(&mut inner_nodes, override_weight, override_slope);
        let node = MathNode::Group(inner_nodes);
        return Some((ParsedStep::Atom { node, is_roman_op: false }, consumed));
    }

    if let Some(op) = lookup_roman_operator(name) {
        return Some((
            ParsedStep::Atom {
                node: MathNode::Text { text: op.to_owned(), weight: SpanWeight::Regular, slope: SpanSlope::Upright, is_single_char: false },
                is_roman_op: true,
            },
            0,
        ));
    }

    if name == "quad" {
        return Some((ParsedStep::Raw(MathNode::Kern { width_em: 1.0 }), 0));
    }
    if name == "qquad" {
        return Some((ParsedStep::Raw(MathNode::Kern { width_em: 2.0 }), 0));
    }
    if name == "space" {
        return Some((ParsedStep::Raw(MathNode::Space { width_em: 0.278 }), 0));
    }

    if let Some(sym) = lookup_latex_escape(name) {
        let empty_group =
            if matches!(name, "textbackslash" | "textless" | "textgreater" | "textdegree" | "percent") && tail.starts_with("{}") { 2 } else { 0 };
        return Some((
            ParsedStep::Atom {
                node: MathNode::Text { text: sym.to_owned(), weight: SpanWeight::Regular, slope: SpanSlope::Upright, is_single_char: true },
                is_roman_op: false,
            },
            empty_group,
        ));
    }

    None
}

fn apply_style_override(nodes: &mut [MathNode], weight: Option<SpanWeight>, slope: Option<SpanSlope>) {
    for node in nodes {
        match node {
            MathNode::Text { weight: w, slope: s, is_single_char, .. } => {
                if let Some(nw) = weight {
                    *w = nw;
                }
                if let Some(ns) = slope {
                    *s = ns;
                }
                *is_single_char = false;
            }
            MathNode::Group(inner) => apply_style_override(inner, weight, slope),
            MathNode::Scripted { nucleus, sup, sub } => {
                apply_style_override(std::slice::from_mut(nucleus.as_mut()), weight, slope);
                if let Some(sp) = sup {
                    apply_style_override(sp, weight, slope);
                }
                if let Some(sb) = sub {
                    apply_style_override(sb, weight, slope);
                }
            }
            _ => {}
        }
    }
}

fn try_parse_html_step(rest: &str, text_mode: bool, depth: usize) -> Option<(MathNode, usize)> {
    let close = rest.find('>')?;
    if close == 0 || close > 8 {
        return None;
    }
    let tag = rest.get(1..close)?;
    let (end_tag, script_kind) = if tag.eq_ignore_ascii_case("b") {
        ("</b>", None)
    } else if tag.eq_ignore_ascii_case("i") {
        ("</i>", None)
    } else if tag.eq_ignore_ascii_case("sup") {
        ("</sup>", Some(SpanScript::Superscript))
    } else if tag.eq_ignore_ascii_case("sub") {
        ("</sub>", Some(SpanScript::Subscript))
    } else {
        return None;
    };
    let after_open = rest.get(close + 1..)?;
    let close_pos = find_ascii_case_insensitive(after_open, end_tag)?;
    let inner = after_open.get(..close_pos)?;
    let total = close + 1 + close_pos + end_tag.len();
    let mut inner_nodes = parse_math_slice(inner, text_mode, depth + 1);
    match script_kind {
        Some(SpanScript::Superscript) => {
            Some((MathNode::Scripted { nucleus: Box::new(MathNode::Group(Vec::new())), sup: Some(inner_nodes), sub: None }, total))
        }
        Some(SpanScript::Subscript) => {
            Some((MathNode::Scripted { nucleus: Box::new(MathNode::Group(Vec::new())), sup: None, sub: Some(inner_nodes) }, total))
        }
        _ => {
            let weight = tag.eq_ignore_ascii_case("b").then_some(SpanWeight::Bold);
            let slope = tag.eq_ignore_ascii_case("i").then_some(SpanSlope::Italic);
            apply_style_override(&mut inner_nodes, weight, slope);
            Some((MathNode::Group(inner_nodes), total))
        }
    }
}

fn find_ascii_case_insensitive(haystack: &str, needle: &str) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.char_indices().find_map(|(idx, _)| haystack.get(idx..idx + needle.len()).and_then(|sub| sub.eq_ignore_ascii_case(needle).then_some(idx)))
}

fn attach_scripts_if_any(mut nucleus: MathNode, input: &str, pos: &mut usize, text_mode: bool, depth: usize) -> MathNode {
    if depth >= MAX_TEX_DEPTH {
        return nucleus;
    }
    let mut sup: Option<Vec<MathNode>> = None;
    let mut sub: Option<Vec<MathNode>> = None;

    while let Some(rest) = input.get(*pos..) {
        let Some(marker) = rest.chars().next() else {
            break;
        };
        if marker != '^' && marker != '_' {
            break;
        }
        let after = rest.get(1..).unwrap_or("");
        let Some((arg_nodes, arg_consumed)) = parse_script_arg(after, text_mode, depth + 1) else {
            break;
        };
        *pos += 1 + arg_consumed;
        if marker == '^' {
            if sup.is_some() {
                nucleus = MathNode::Scripted { nucleus: Box::new(nucleus), sup: sup.take(), sub: sub.take() };
            }
            sup = Some(arg_nodes);
        } else {
            if sub.is_some() {
                nucleus = MathNode::Scripted { nucleus: Box::new(nucleus), sup: sup.take(), sub: sub.take() };
            }
            sub = Some(arg_nodes);
        }
    }

    if sup.is_some() || sub.is_some() { MathNode::Scripted { nucleus: Box::new(nucleus), sup, sub } } else { nucleus }
}

fn parse_script_arg(s: &str, text_mode: bool, depth: usize) -> Option<(Vec<MathNode>, usize)> {
    let trimmed = s.trim_start_matches([' ', '\t']);
    let ws = s.len() - trimmed.len();
    let first = trimmed.chars().next()?;
    if first == '}' {
        return None;
    }
    if first == '{' {
        let (inner, consumed) = extract_braced(trimmed)?;
        let mut nodes = parse_math_slice(inner, text_mode, depth + 1);
        trim_edge_spaces(&mut nodes);
        return Some((nodes, ws + consumed));
    }
    if first == '\\' {
        let mut local_pos = 0usize;
        let step = parse_backslash_step(trimmed, &mut local_pos, depth + 1);
        let node = match step {
            ParsedStep::Atom { node, .. } | ParsedStep::Raw(node) => node,
        };
        return Some((vec![node], ws + local_pos));
    }
    let mut local_pos = 0usize;
    let step = parse_literal_atom(trimmed, &mut local_pos, text_mode);
    let node = match step {
        ParsedStep::Atom { node, .. } | ParsedStep::Raw(node) => node,
    };
    Some((vec![node], ws + local_pos))
}

fn needs_operator_thin_space(after: &str) -> bool {
    let trimmed = after.trim_start_matches([' ', '\t']);
    if after.len() > trimmed.len() {
        return false; // Explicit space already follows.
    }
    let Some(first) = trimmed.chars().next() else {
        return false;
    };
    if matches!(first, '(' | '[' | '|' | ',' | '.' | ';' | ':' | ')' | ']' | '}' | '^' | '_') {
        return false;
    }
    !trimmed.starts_with("\\left") && !trimmed.starts_with("\\right")
}

fn trim_edge_spaces(nodes: &mut Vec<MathNode>) {
    while matches!(nodes.first(), Some(MathNode::Space { .. })) {
        nodes.remove(0);
    }
    while matches!(nodes.last(), Some(MathNode::Space { .. })) {
        nodes.pop();
    }
}

fn extract_one_math_arg(s: &str) -> Option<(&str, usize)> {
    let trimmed = s.trim_start_matches([' ', '\t']);
    let ws = s.len() - trimmed.len();
    if trimmed.starts_with('{') {
        let (inner, consumed) = extract_braced(trimmed)?;
        return Some((inner, ws + consumed));
    }
    let ch = trimmed.chars().next()?;
    if ch == '\\' {
        let after = trimmed.get(1..)?;
        let nlen = after.bytes().take_while(u8::is_ascii_alphabetic).count();
        let end = if nlen > 0 { 1 + nlen } else { 1 + after.chars().next().map_or(0, char::len_utf8) };
        return Some((trimmed.get(..end)?, ws + end));
    }
    Some((trimmed.get(..ch.len_utf8())?, ws + ch.len_utf8()))
}

fn extract_two_math_args(s: &str) -> Option<(&str, &str, usize)> {
    let (a, ca) = extract_one_math_arg(s)?;
    let (b, cb) = extract_one_math_arg(s.get(ca..)?)?;
    Some((a, b, ca + cb))
}

fn extract_sqrt_args(s: &str) -> Option<(Option<&str>, &str, usize)> {
    let trimmed = s.trim_start_matches([' ', '\t']);
    let ws = s.len() - trimmed.len();
    if let Some(after_bracket) = trimmed.strip_prefix('[')
        && let Some(close_idx) = find_matching_square_bracket(after_bracket)
    {
        let deg = after_bracket.get(..close_idx)?;
        let rest_after_deg = after_bracket.get(close_idx + 1..)?;
        let (rad, rad_consumed) = extract_one_math_arg(rest_after_deg)?;
        return Some((Some(deg), rad, ws + 1 + close_idx + 1 + rad_consumed));
    }
    let (rad, rad_consumed) = extract_one_math_arg(s)?;
    Some((None, rad, rad_consumed))
}

fn find_matching_square_bracket(s: &str) -> Option<usize> {
    let mut brace_depth = 0usize;
    let mut sq_depth = 0usize;
    let mut i = 0usize;
    while i < s.len() {
        let ch = s.get(i..)?.chars().next()?;
        if ch == '\\' {
            i += 1;
            if let Some(nc) = s.get(i..).and_then(|t| t.chars().next()) {
                i += nc.len_utf8();
            }
            continue;
        }
        match ch {
            '{' => brace_depth = brace_depth.saturating_add(1),
            '}' => brace_depth = brace_depth.saturating_sub(1),
            '[' if brace_depth == 0 => sq_depth = sq_depth.saturating_add(1),
            ']' if brace_depth == 0 => {
                if sq_depth == 0 {
                    return Some(i);
                }
                sq_depth -= 1;
            }
            _ => {}
        }
        i += ch.len_utf8();
    }
    None
}

fn parse_tex_delim_token(s: &str) -> Option<(DelimKind, usize)> {
    let (delim, consumed) = parse_tex_delimiter_token(s)?;
    let kind = match delim {
        TexDelimiter::LeftParen | TexDelimiter::RightParen => DelimKind::Paren,
        TexDelimiter::LeftBracket | TexDelimiter::RightBracket => DelimKind::Bracket,
        TexDelimiter::LeftBrace | TexDelimiter::RightBrace => DelimKind::Brace,
        TexDelimiter::Pipe => DelimKind::Pipe,
        TexDelimiter::DoublePipe => DelimKind::DoublePipe,
        TexDelimiter::Null => DelimKind::Null,
    };
    Some((kind, consumed))
}

fn extract_left_right_pair(tail: &str) -> Option<(DelimKind, &str, DelimKind, usize)> {
    let (left_delim, left_len) = parse_tex_delim_token(tail)?;
    let body = tail.get(left_len..)?;
    let mut depth = 0usize;
    let mut brace_depth = 0usize;
    let mut i = 0usize;
    while i < body.len() {
        let rest = body.get(i..)?;
        if rest.starts_with('\\') {
            if brace_depth == 0
                && rest.starts_with("\\left")
                && !rest.get(5..).and_then(|s| s.chars().next()).is_some_and(|c| c.is_ascii_alphabetic())
            {
                depth = depth.saturating_add(1);
                i += 5;
                continue;
            }
            if brace_depth == 0
                && rest.starts_with("\\right")
                && !rest.get(6..).and_then(|s| s.chars().next()).is_some_and(|c| c.is_ascii_alphabetic())
            {
                if depth == 0 {
                    let after_right = rest.get(6..)?;
                    let (right_delim, right_len) = parse_tex_delim_token(after_right)?;
                    return Some((left_delim, body.get(..i)?, right_delim, left_len + i + 6 + right_len));
                }
                depth -= 1;
                i += 6;
                continue;
            }
            i += 1;
            if let Some(nc) = body.get(i..).and_then(|s| s.chars().next()) {
                i += nc.len_utf8();
            }
            continue;
        }
        let ch = rest.chars().next()?;
        if ch == '{' {
            brace_depth = brace_depth.saturating_add(1);
        } else if ch == '}' {
            brace_depth = brace_depth.saturating_sub(1);
        }
        i += ch.len_utf8();
    }
    None
}

fn try_parse_matched_delim(rest: &str, open_ch: char, depth: usize) -> Option<(Vec<MathNode>, usize)> {
    if depth >= MAX_TEX_DEPTH {
        return None;
    }
    let close_ch = match open_ch {
        '(' => ')',
        '[' => ']',
        '|' => '|',
        _ => return None,
    };
    let body = rest.get(open_ch.len_utf8()..)?;
    let mut brace_depth = 0usize;
    let mut left_depth = 0usize;
    let mut paren_depth = 0usize;
    let mut sq_depth = 0usize;
    let mut i = 0usize;

    while i < body.len() {
        let cur = body.get(i..)?;
        if cur.starts_with('\\') {
            if brace_depth == 0 && cur.starts_with("\\left") && !cur.get(5..).and_then(|s| s.chars().next()).is_some_and(|c| c.is_ascii_alphabetic())
            {
                left_depth = left_depth.saturating_add(1);
                i += 5;
                continue;
            }
            if brace_depth == 0 && cur.starts_with("\\right") && !cur.get(6..).and_then(|s| s.chars().next()).is_some_and(|c| c.is_ascii_alphabetic())
            {
                left_depth = left_depth.saturating_sub(1);
                i += 6;
                continue;
            }
            i += 1;
            if let Some(nc) = body.get(i..).and_then(|s| s.chars().next()) {
                i += nc.len_utf8();
            }
            continue;
        }
        let ch = cur.chars().next()?;
        if ch == '{' {
            brace_depth = brace_depth.saturating_add(1);
        } else if ch == '}' {
            if brace_depth == 0 {
                return None;
            }
            brace_depth -= 1;
        } else if brace_depth == 0 && left_depth == 0 {
            if open_ch == '|' {
                match ch {
                    '(' => paren_depth = paren_depth.saturating_add(1),
                    ')' => {
                        if paren_depth == 0 {
                            return None;
                        }
                        paren_depth -= 1;
                    }
                    '[' => sq_depth = sq_depth.saturating_add(1),
                    ']' => {
                        if sq_depth == 0 {
                            return None;
                        }
                        sq_depth -= 1;
                    }
                    '|' if paren_depth == 0 && sq_depth == 0 => {
                        let inner = body.get(..i)?;
                        let mut inner_nodes = parse_math_slice(inner, false, depth + 1);
                        trim_edge_spaces(&mut inner_nodes);
                        return Some((inner_nodes, open_ch.len_utf8() + i + close_ch.len_utf8()));
                    }
                    _ => {}
                }
            } else if ch == open_ch {
                paren_depth = paren_depth.saturating_add(1);
            } else if ch == close_ch {
                if paren_depth == 0 {
                    let inner = body.get(..i)?;
                    let mut inner_nodes = parse_math_slice(inner, false, depth + 1);
                    trim_edge_spaces(&mut inner_nodes);
                    return Some((inner_nodes, open_ch.len_utf8() + i + close_ch.len_utf8()));
                }
                paren_depth -= 1;
            }
        }
        i += ch.len_utf8();
    }
    None
}
