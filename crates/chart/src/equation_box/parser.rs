//! Structured TeX-subset math AST and recursive-descent parser for live model equation boxes.
//!
//! Parses nested fractions, superscripts/subscripts, radicals, scaled delimiters, Greek
//! identifiers, and roman math operators following the math-list grammar of Knuth, D. E. (1984),
//! *The TeXbook*, Addison-Wesley, Chapter 17 (pp. 140–145).

use std::collections::HashMap;

use curvebook_text::{
    SpanScript, SpanSlope, SpanWeight, TexDelimiter, lookup_latex_escape, lookup_roman_operator, match_balanced_math_delimiters,
    parse_tex_delimiter_token, unwrap_outer_math_delimiters,
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

#[derive(Debug)]
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

fn drain_math_children(node: &mut MathNode, stack: &mut Vec<MathNode>) {
    match node {
        MathNode::Text { .. } | MathNode::Space { .. } | MathNode::ThinSpace | MathNode::Kern { .. } => {}
        MathNode::Fraction { numerator, denominator, .. } => {
            stack.append(numerator);
            stack.append(denominator);
        }
        MathNode::Radical { degree, radicand } => {
            if let Some(deg) = degree.take() {
                stack.extend(deg);
            }
            stack.append(radicand);
        }
        MathNode::Delimited { inner, .. } | MathNode::Group(inner) => {
            stack.append(inner);
        }
        MathNode::Scripted { nucleus, sup, sub } => {
            if let Some(sp) = sup.take() {
                stack.extend(sp);
            }
            if let Some(sb) = sub.take() {
                stack.extend(sb);
            }
            if !matches!(nucleus.as_ref(), MathNode::ThinSpace) {
                let inner_nuc = std::mem::replace(nucleus.as_mut(), MathNode::ThinSpace);
                stack.push(inner_nuc);
            }
        }
    }
}

impl Drop for MathNode {
    fn drop(&mut self) {
        let mut stack: Vec<MathNode> = Vec::new();
        drain_math_children(self, &mut stack);
        while let Some(mut node) = stack.pop() {
            drain_math_children(&mut node, &mut stack);
        }
    }
}

impl Clone for MathNode {
    fn clone(&self) -> Self {
        enum CloneTask<'a> {
            Visit(&'a MathNode),
            BuildFraction { num_len: usize, den_len: usize, kind: FractionKind },
            BuildRadical { deg_len: Option<usize>, rad_len: usize },
            BuildDelimited { left: DelimKind, inner_len: usize, right: DelimKind, explicit: bool },
            BuildGroup(usize),
            BuildScripted { sup_len: Option<usize>, sub_len: Option<usize> },
        }

        let mut tasks = vec![CloneTask::Visit(self)];
        let mut out: Vec<MathNode> = Vec::new();

        while let Some(task) = tasks.pop() {
            match task {
                CloneTask::Visit(node) => match node {
                    MathNode::Text { text, weight, slope, is_single_char } => {
                        out.push(MathNode::Text { text: text.clone(), weight: *weight, slope: *slope, is_single_char: *is_single_char });
                    }
                    MathNode::Space { width_em } => out.push(MathNode::Space { width_em: *width_em }),
                    MathNode::ThinSpace => out.push(MathNode::ThinSpace),
                    MathNode::Kern { width_em } => out.push(MathNode::Kern { width_em: *width_em }),
                    MathNode::Fraction { numerator, denominator, kind } => {
                        tasks.push(CloneTask::BuildFraction { num_len: numerator.len(), den_len: denominator.len(), kind: *kind });
                        for child in denominator.iter().rev() {
                            tasks.push(CloneTask::Visit(child));
                        }
                        for child in numerator.iter().rev() {
                            tasks.push(CloneTask::Visit(child));
                        }
                    }
                    MathNode::Radical { degree, radicand } => {
                        tasks.push(CloneTask::BuildRadical { deg_len: degree.as_ref().map(Vec::len), rad_len: radicand.len() });
                        for child in radicand.iter().rev() {
                            tasks.push(CloneTask::Visit(child));
                        }
                        if let Some(deg) = degree {
                            for child in deg.iter().rev() {
                                tasks.push(CloneTask::Visit(child));
                            }
                        }
                    }
                    MathNode::Delimited { left, inner, right, explicit } => {
                        tasks.push(CloneTask::BuildDelimited { left: *left, inner_len: inner.len(), right: *right, explicit: *explicit });
                        for child in inner.iter().rev() {
                            tasks.push(CloneTask::Visit(child));
                        }
                    }
                    MathNode::Group(inner) => {
                        tasks.push(CloneTask::BuildGroup(inner.len()));
                        for child in inner.iter().rev() {
                            tasks.push(CloneTask::Visit(child));
                        }
                    }
                    MathNode::Scripted { nucleus, sup, sub } => {
                        tasks.push(CloneTask::BuildScripted { sup_len: sup.as_ref().map(Vec::len), sub_len: sub.as_ref().map(Vec::len) });
                        if let Some(sb) = sub {
                            for child in sb.iter().rev() {
                                tasks.push(CloneTask::Visit(child));
                            }
                        }
                        if let Some(sp) = sup {
                            for child in sp.iter().rev() {
                                tasks.push(CloneTask::Visit(child));
                            }
                        }
                        tasks.push(CloneTask::Visit(nucleus));
                    }
                },
                CloneTask::BuildFraction { num_len, den_len, kind } => {
                    let den_start = out.len().saturating_sub(den_len);
                    let denominator = out.split_off(den_start);
                    let num_start = out.len().saturating_sub(num_len);
                    let numerator = out.split_off(num_start);
                    out.push(MathNode::Fraction { numerator, denominator, kind });
                }
                CloneTask::BuildRadical { deg_len, rad_len } => {
                    let rad_start = out.len().saturating_sub(rad_len);
                    let radicand = out.split_off(rad_start);
                    let degree = deg_len.map(|n| {
                        let start = out.len().saturating_sub(n);
                        out.split_off(start)
                    });
                    out.push(MathNode::Radical { degree, radicand });
                }
                CloneTask::BuildDelimited { left, inner_len, right, explicit } => {
                    let start = out.len().saturating_sub(inner_len);
                    let inner = out.split_off(start);
                    out.push(MathNode::Delimited { left, inner, right, explicit });
                }
                CloneTask::BuildGroup(len) => {
                    let start = out.len().saturating_sub(len);
                    let inner = out.split_off(start);
                    out.push(MathNode::Group(inner));
                }
                CloneTask::BuildScripted { sup_len, sub_len } => {
                    let sub = sub_len.map(|n| {
                        let start = out.len().saturating_sub(n);
                        out.split_off(start)
                    });
                    let sup = sup_len.map(|n| {
                        let start = out.len().saturating_sub(n);
                        out.split_off(start)
                    });
                    let nucleus = out.pop().unwrap_or(MathNode::ThinSpace);
                    out.push(MathNode::Scripted { nucleus: Box::new(nucleus), sup, sub });
                }
            }
        }

        out.pop().unwrap_or(MathNode::ThinSpace)
    }
}

pub(super) enum ParsedStep {
    Atom { node: MathNode, is_roman_op: bool },
    Raw(MathNode),
}

// ---------------------------------------------------------------------------
// Single-Pass O(N) Delimiter Match Table
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
struct LeftRightMatch {
    left_delim: DelimKind,
    body_start: usize,
    body_end: usize,
    right_delim: DelimKind,
    total_end: usize,
}

#[derive(Debug, Default)]
struct ScanIndex {
    braces: HashMap<usize, usize>,
    parens: HashMap<usize, usize>,
    brackets: HashMap<usize, usize>,
    sqrt_brackets: HashMap<usize, usize>,
    pipes: HashMap<usize, usize>,
    left_right: HashMap<usize, LeftRightMatch>,
    /// Sorted byte offsets of every `$` (ascending, as scanned).
    dollars: Vec<usize>,
    /// Sorted byte offsets of every `$$` start.
    double_dollars: Vec<usize>,
}

#[derive(Debug, Clone, Copy)]
struct LeftStackEntry {
    cmd_start: usize,
    body_start: usize,
    left_delim: DelimKind,
    brace_depth: usize,
    saved_paren_depth: usize,
    saved_sq_depth: usize,
}

#[derive(Debug, Clone, Copy)]
struct BraceStackEntry {
    open_pos: usize,
    saved_left_depth: usize,
    saved_paren_depth: usize,
    saved_sq_depth: usize,
}

#[derive(Debug, Clone, Copy)]
struct DelimStackEntry {
    open_pos: usize,
    brace_depth: usize,
    left_depth: usize,
}

#[derive(Debug, Clone, Copy)]
struct PipeStackEntry {
    open_pos: usize,
    brace_depth: usize,
    left_depth: usize,
    paren_depth: usize,
    sq_depth: usize,
}

impl ScanIndex {
    /// Precompute balanced delimiter pairs in a single linear O(N) pass so unmatched `(`, `[`, `|`,
    /// `{`, `\left`, and `\sqrt[` never trigger quadratic rescans to EOF (`GAP-01-02`).
    fn build(source: &str) -> Self {
        let mut index = Self::default();
        let mut brace_stack: Vec<BraceStackEntry> = Vec::new();
        let mut left_stack: Vec<LeftStackEntry> = Vec::new();
        let mut paren_stack: Vec<DelimStackEntry> = Vec::new();
        let mut bracket_stack: Vec<DelimStackEntry> = Vec::new();
        let mut sqrt_bracket_stack: Vec<(usize, usize)> = Vec::new();
        let mut pipe_stack: Vec<PipeStackEntry> = Vec::new();

        let mut brace_depth = 0usize;
        let mut left_depth = 0usize;
        let mut paren_depth = 0usize;
        let mut sq_depth = 0usize;
        let mut i = 0usize;

        while i < source.len() {
            let Some(rest) = source.get(i..) else {
                break;
            };
            let Some(ch) = rest.chars().next() else {
                break;
            };
            if ch == '\\' {
                if rest.starts_with("\\left")
                    && !rest.get(5..).and_then(|s| s.chars().next()).is_some_and(|c| c.is_ascii_alphabetic())
                    && let Some(after_left) = rest.get(5..)
                    && let Some((left_delim, left_len)) = parse_tex_delim_token(after_left)
                {
                    let body_start = i + 5 + left_len;
                    left_stack.push(LeftStackEntry {
                        cmd_start: i,
                        body_start,
                        left_delim,
                        brace_depth,
                        saved_paren_depth: paren_depth,
                        saved_sq_depth: sq_depth,
                    });
                    left_depth = left_depth.saturating_add(1);
                    paren_depth = 0;
                    sq_depth = 0;
                    i = body_start;
                    continue;
                }
                if rest.starts_with("\\right")
                    && !rest.get(6..).and_then(|s| s.chars().next()).is_some_and(|c| c.is_ascii_alphabetic())
                    && let Some(after_right) = rest.get(6..)
                    && let Some((right_delim, right_len)) = parse_tex_delim_token(after_right)
                {
                    let total_end = i + 6 + right_len;
                    if left_stack.last().is_some_and(|top| top.brace_depth == brace_depth)
                        && let Some(open) = left_stack.pop()
                    {
                        while paren_stack.last().is_some_and(|e| e.brace_depth == brace_depth && e.left_depth == left_depth) {
                            paren_stack.pop();
                        }
                        while bracket_stack.last().is_some_and(|e| e.brace_depth == brace_depth && e.left_depth == left_depth) {
                            bracket_stack.pop();
                        }
                        while pipe_stack.last().is_some_and(|e| e.brace_depth == brace_depth && e.left_depth == left_depth) {
                            pipe_stack.pop();
                        }
                        index.left_right.insert(
                            open.cmd_start,
                            LeftRightMatch { left_delim: open.left_delim, body_start: open.body_start, body_end: i, right_delim, total_end },
                        );
                        left_depth = left_depth.saturating_sub(1);
                        paren_depth = open.saved_paren_depth;
                        sq_depth = open.saved_sq_depth;
                    }
                    i = total_end;
                    continue;
                }
                i += 1;
                if let Some(nc) = source.get(i..).and_then(|s| s.chars().next()) {
                    i += nc.len_utf8();
                }
                continue;
            }

            match ch {
                '$' => {
                    index.dollars.push(i);
                    if rest.starts_with("$$") {
                        index.double_dollars.push(i);
                    }
                    i += 1;
                }
                '{' => {
                    brace_stack.push(BraceStackEntry {
                        open_pos: i,
                        saved_left_depth: left_depth,
                        saved_paren_depth: paren_depth,
                        saved_sq_depth: sq_depth,
                    });
                    brace_depth = brace_depth.saturating_add(1);
                    left_depth = 0;
                    paren_depth = 0;
                    sq_depth = 0;
                    i += 1;
                }
                '}' => {
                    while left_stack.last().is_some_and(|e| e.brace_depth == brace_depth) {
                        left_stack.pop();
                    }
                    while paren_stack.last().is_some_and(|e| e.brace_depth == brace_depth) {
                        paren_stack.pop();
                    }
                    while bracket_stack.last().is_some_and(|e| e.brace_depth == brace_depth) {
                        bracket_stack.pop();
                    }
                    while sqrt_bracket_stack.last().is_some_and(|&(_, bd)| bd == brace_depth) {
                        sqrt_bracket_stack.pop();
                    }
                    while pipe_stack.last().is_some_and(|e| e.brace_depth == brace_depth) {
                        pipe_stack.pop();
                    }
                    if let Some(open) = brace_stack.pop() {
                        index.braces.insert(open.open_pos, i);
                        brace_depth = brace_depth.saturating_sub(1);
                        left_depth = open.saved_left_depth;
                        paren_depth = open.saved_paren_depth;
                        sq_depth = open.saved_sq_depth;
                    } else {
                        left_depth = 0;
                        paren_depth = 0;
                        sq_depth = 0;
                    }
                    i += 1;
                }
                '(' => {
                    paren_stack.push(DelimStackEntry { open_pos: i, brace_depth, left_depth });
                    paren_depth = paren_depth.saturating_add(1);
                    i += 1;
                }
                ')' => {
                    if paren_stack.last().is_some_and(|e| e.brace_depth == brace_depth && e.left_depth == left_depth)
                        && let Some(open) = paren_stack.pop()
                    {
                        index.parens.insert(open.open_pos, i);
                        paren_depth = paren_depth.saturating_sub(1);
                        while pipe_stack
                            .last()
                            .is_some_and(|e| e.brace_depth == brace_depth && e.left_depth == left_depth && e.paren_depth > paren_depth)
                        {
                            pipe_stack.pop();
                        }
                    } else {
                        while pipe_stack.last().is_some_and(|e| e.brace_depth == brace_depth && e.left_depth == left_depth) {
                            pipe_stack.pop();
                        }
                    }
                    i += 1;
                }
                '[' => {
                    bracket_stack.push(DelimStackEntry { open_pos: i, brace_depth, left_depth });
                    sqrt_bracket_stack.push((i, brace_depth));
                    sq_depth = sq_depth.saturating_add(1);
                    i += 1;
                }
                ']' => {
                    if sqrt_bracket_stack.last().is_some_and(|&(_, bd)| bd == brace_depth)
                        && let Some((open_pos, _)) = sqrt_bracket_stack.pop()
                    {
                        index.sqrt_brackets.insert(open_pos, i);
                    }
                    if bracket_stack.last().is_some_and(|e| e.brace_depth == brace_depth && e.left_depth == left_depth)
                        && let Some(open) = bracket_stack.pop()
                    {
                        index.brackets.insert(open.open_pos, i);
                        sq_depth = sq_depth.saturating_sub(1);
                        while pipe_stack.last().is_some_and(|e| e.brace_depth == brace_depth && e.left_depth == left_depth && e.sq_depth > sq_depth) {
                            pipe_stack.pop();
                        }
                    } else {
                        while pipe_stack.last().is_some_and(|e| e.brace_depth == brace_depth && e.left_depth == left_depth) {
                            pipe_stack.pop();
                        }
                    }
                    i += 1;
                }
                '|' => {
                    if pipe_stack.last().is_some_and(|e| {
                        e.brace_depth == brace_depth && e.left_depth == left_depth && e.paren_depth == paren_depth && e.sq_depth == sq_depth
                    }) && let Some(open) = pipe_stack.pop()
                    {
                        index.pipes.insert(open.open_pos, i);
                    } else {
                        pipe_stack.push(PipeStackEntry { open_pos: i, brace_depth, left_depth, paren_depth, sq_depth });
                    }
                    i += 1;
                }
                _ => {
                    i += ch.len_utf8();
                }
            }
        }
        index
    }

    fn match_brace<'a>(&self, slice: &'a str, rel_pos: usize, base_offset: usize) -> Option<(&'a str, usize)> {
        let rest = slice.get(rel_pos..)?;
        if !rest.starts_with('{') {
            return None;
        }
        let open_abs = base_offset + rel_pos;
        let close_abs = *self.braces.get(&open_abs)?;
        let slice_end = base_offset + slice.len();
        if close_abs < open_abs + 1 || close_abs >= slice_end {
            return None;
        }
        let close_rel = close_abs - base_offset;
        let inner = slice.get(rel_pos + 1..close_rel)?;
        Some((inner, close_rel - rel_pos + 1))
    }

    fn match_braced_opt_ws<'a>(&self, slice: &'a str, rel_pos: usize, base_offset: usize) -> Option<(&'a str, usize, usize)> {
        let rest = slice.get(rel_pos..)?;
        let trimmed = rest.trim_start_matches([' ', '\t']);
        let ws = rest.len() - trimmed.len();
        let (inner, consumed) = self.match_brace(slice, rel_pos + ws, base_offset)?;
        Some((inner, rel_pos + ws + 1, ws + consumed))
    }

    fn match_delim(&self, open_ch: char, open_abs: usize, slice_end: usize) -> Option<usize> {
        let table = match open_ch {
            '(' => &self.parens,
            '[' => &self.brackets,
            '|' => &self.pipes,
            _ => return None,
        };
        let close_abs = *table.get(&open_abs)?;
        (close_abs > open_abs && close_abs < slice_end).then_some(close_abs)
    }

    fn match_sqrt_bracket(&self, open_abs: usize, slice_end: usize) -> Option<usize> {
        let close_abs = *self.sqrt_brackets.get(&open_abs)?;
        (close_abs > open_abs && close_abs < slice_end).then_some(close_abs)
    }

    fn match_left_right(&self, cmd_abs: usize, slice_end: usize) -> Option<LeftRightMatch> {
        let matched = *self.left_right.get(&cmd_abs)?;
        (matched.total_end <= slice_end && matched.body_start <= matched.body_end).then_some(matched)
    }

    /// Whether a closing delimiter of `delim_len` could exist inside `[abs_pos + delim_len, slice_end]`.
    /// Binary search over all recorded delimiter positions, so a `$` after the current group cannot hide one inside it.
    fn has_later_dollar(&self, delim_len: usize, abs_pos: usize, slice_end: usize) -> bool {
        let positions = if delim_len == 2 { &self.double_dollars } else { &self.dollars };
        let from = abs_pos + delim_len;
        let first = positions.partition_point(|&idx| idx < from);
        positions.get(first).is_some_and(|&idx| idx + delim_len <= slice_end)
    }
}

#[derive(Debug, Clone, Copy, Default)]
enum TagCloseCache {
    #[default]
    Unknown,
    None,
    At(usize),
}

#[derive(Debug, Default)]
struct SliceState {
    html_close_cache: [TagCloseCache; 4],
}

impl SliceState {
    fn find_html_close(&mut self, slice: &str, tag_idx: usize, search_from: usize, end_tag: &str) -> Option<usize> {
        match self.html_close_cache[tag_idx] {
            TagCloseCache::None => None,
            TagCloseCache::At(found) if found >= search_from => Some(found - search_from),
            _ => {
                let tail = slice.get(search_from..)?;
                match find_ascii_case_insensitive(tail, end_tag) {
                    Some(rel) => {
                        self.html_close_cache[tag_idx] = TagCloseCache::At(search_from + rel);
                        Some(rel)
                    }
                    None => {
                        self.html_close_cache[tag_idx] = TagCloseCache::None;
                        None
                    }
                }
            }
        }
    }
}

#[derive(Clone, Copy)]
struct ParseCursor<'a> {
    slice: &'a str,
    base_offset: usize,
    text_mode: bool,
    depth: usize,
    scan: &'a ScanIndex,
}

pub(super) fn parse_math_nodes(input: &str) -> Vec<MathNode> {
    let trimmed = input.trim();
    let source = unwrap_outer_math_delimiters(trimmed).unwrap_or(input);
    let scan = ScanIndex::build(source);
    let mut nodes = parse_math_slice_indexed(source, 0, false, 0, &scan);
    trim_edge_spaces(&mut nodes);
    nodes
}

#[cfg(test)]
pub(super) fn parse_math_slice(input: &str, text_mode: bool, depth: usize) -> Vec<MathNode> {
    let scan = ScanIndex::build(input);
    parse_math_slice_indexed(input, 0, text_mode, depth, &scan)
}

fn parse_math_slice_indexed(slice: &str, base_offset: usize, text_mode: bool, depth: usize, scan: &ScanIndex) -> Vec<MathNode> {
    if depth >= MAX_TEX_DEPTH {
        if slice.is_empty() {
            return Vec::new();
        }
        return vec![MathNode::Text { text: slice.to_owned(), weight: SpanWeight::Regular, slope: SpanSlope::Upright, is_single_char: false }];
    }

    let cursor = ParseCursor { slice, base_offset, text_mode, depth, scan };
    let mut state = SliceState::default();
    let mut nodes = Vec::new();
    let mut pos = 0usize;
    while pos < slice.len() {
        let prev_pos = pos;
        let Some(rest) = slice.get(pos..) else {
            break;
        };
        let Some(ch) = rest.chars().next() else {
            break;
        };
        if ch.is_whitespace() {
            while let Some(c) = slice.get(pos..).and_then(|s| s.chars().next())
                && c.is_whitespace()
            {
                pos += c.len_utf8();
            }
            if !matches!(nodes.last(), Some(MathNode::Space { .. })) {
                nodes.push(MathNode::Space { width_em: 0.278 });
            }
            continue;
        }
        if let Some(step) = parse_next_step_indexed(cursor, &mut pos, &mut state) {
            match step {
                ParsedStep::Raw(node) => nodes.push(node),
                ParsedStep::Atom { node, is_roman_op } => {
                    let attached = attach_scripts_if_any(node, cursor, &mut pos);
                    nodes.push(attached);
                    if is_roman_op && needs_operator_thin_space(slice.get(pos..).unwrap_or("")) {
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

#[cfg(test)]
pub(super) fn parse_next_step(input: &str, pos: &mut usize, text_mode: bool, depth: usize) -> Option<ParsedStep> {
    let scan = ScanIndex::build(input);
    let mut state = SliceState::default();
    parse_next_step_indexed(ParseCursor { slice: input, base_offset: 0, text_mode, depth, scan: &scan }, pos, &mut state)
}

fn parse_next_step_indexed(cursor: ParseCursor<'_>, pos: &mut usize, state: &mut SliceState) -> Option<ParsedStep> {
    let rest = cursor.slice.get(*pos..)?;
    let ch = rest.chars().next()?;
    let slice_end = cursor.base_offset + cursor.slice.len();

    if ch == '<'
        && let Some((node, consumed)) = try_parse_html_step(cursor, *pos, state)
    {
        *pos += consumed;
        return Some(ParsedStep::Atom { node, is_roman_op: false });
    }
    if ch == '\\' {
        return Some(parse_backslash_step(cursor, pos));
    }
    if ch == '$' {
        let delim_len = if rest.starts_with("$$") { 2 } else { 1 };
        if cursor.scan.has_later_dollar(delim_len, cursor.base_offset + *pos, slice_end)
            && let Some((inner, consumed)) = match_balanced_math_delimiters(rest, delim_len)
        {
            let inner_base = cursor.base_offset + *pos + delim_len;
            *pos += consumed;
            let mut inner_nodes = parse_math_slice_indexed(inner, inner_base, false, cursor.depth + 1, cursor.scan);
            trim_edge_spaces(&mut inner_nodes);
            return Some(ParsedStep::Atom { node: MathNode::Group(inner_nodes), is_roman_op: false });
        }
    }
    if ch == '{'
        && let Some((inner, consumed)) = cursor.scan.match_brace(cursor.slice, *pos, cursor.base_offset)
    {
        let inner_base = cursor.base_offset + *pos + 1;
        *pos += consumed;
        let mut inner_nodes = parse_math_slice_indexed(inner, inner_base, cursor.text_mode, cursor.depth + 1, cursor.scan);
        trim_edge_spaces(&mut inner_nodes);
        return Some(ParsedStep::Atom { node: MathNode::Group(inner_nodes), is_roman_op: false });
    }
    if !cursor.text_mode && matches!(ch, '(' | '[' | '|') && cursor.depth < MAX_TEX_DEPTH {
        let delim = match ch {
            '(' => DelimKind::Paren,
            '[' => DelimKind::Bracket,
            _ => DelimKind::Pipe,
        };
        let open_abs = cursor.base_offset + *pos;
        if let Some(close_abs) = cursor.scan.match_delim(ch, open_abs, slice_end) {
            let open_len = ch.len_utf8();
            let close_rel = close_abs - cursor.base_offset;
            if let Some(inner) = cursor.slice.get(*pos + open_len..close_rel) {
                let inner_base = open_abs + open_len;
                let mut inner_nodes = parse_math_slice_indexed(inner, inner_base, false, cursor.depth + 1, cursor.scan);
                trim_edge_spaces(&mut inner_nodes);
                *pos = close_rel + open_len;
                return Some(ParsedStep::Atom {
                    node: MathNode::Delimited { left: delim, inner: inner_nodes, right: delim, explicit: false },
                    is_roman_op: false,
                });
            }
        }
    }
    if ch == '^' || ch == '_' {
        // Standalone script marker without a preceding nucleus: attach to an empty nucleus
        // only when a valid script argument follows; otherwise consume the orphan marker literally.
        let start_pos = *pos;
        let empty = MathNode::Group(Vec::new());
        let attached = attach_scripts_if_any(empty, cursor, pos);
        if *pos > start_pos {
            return Some(ParsedStep::Raw(attached));
        }
    }
    Some(parse_literal_atom(cursor.slice, pos, cursor.text_mode))
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

fn parse_backslash_step(cursor: ParseCursor<'_>, pos: &mut usize) -> ParsedStep {
    let rest = cursor.slice.get(*pos..).unwrap_or("");
    let Some(after) = rest.get(1..) else {
        *pos = cursor.slice.len();
        return ParsedStep::Raw(MathNode::Group(Vec::new()));
    };
    let Some(first) = after.chars().next() else {
        *pos = cursor.slice.len();
        return ParsedStep::Raw(MathNode::Group(Vec::new()));
    };

    if first.is_ascii_alphabetic() {
        let name_len = after.bytes().take_while(u8::is_ascii_alphabetic).count();
        let name = after.get(..name_len).unwrap_or("");
        let tail_rel = *pos + 1 + name_len;
        if let Some((step, tail_consumed)) = parse_named_command(cursor, *pos, name, tail_rel) {
            *pos = tail_rel + tail_consumed;
            return step;
        }
        *pos = tail_rel;
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

fn parse_named_command(cursor: ParseCursor<'_>, cmd_rel: usize, name: &str, tail_rel: usize) -> Option<(ParsedStep, usize)> {
    let tail = cursor.slice.get(tail_rel..).unwrap_or("");
    let slice_end = cursor.base_offset + cursor.slice.len();

    if matches!(name, "frac" | "dfrac" | "tfrac" | "binom")
        && let Some(((num_raw, num_rel), (den_raw, den_rel), consumed)) = extract_two_math_args(cursor, tail_rel)
    {
        let kind = match name {
            "dfrac" => FractionKind::Display,
            "tfrac" => FractionKind::Text,
            "binom" => FractionKind::Binomial,
            _ => FractionKind::Auto,
        };
        let mut numerator = parse_math_slice_indexed(num_raw, cursor.base_offset + num_rel, false, cursor.depth + 1, cursor.scan);
        let mut denominator = parse_math_slice_indexed(den_raw, cursor.base_offset + den_rel, false, cursor.depth + 1, cursor.scan);
        trim_edge_spaces(&mut numerator);
        trim_edge_spaces(&mut denominator);
        return Some((ParsedStep::Atom { node: MathNode::Fraction { numerator, denominator, kind }, is_roman_op: false }, consumed));
    }

    if name == "sqrt"
        && let Some((deg_opt, (rad_raw, rad_rel), consumed)) = extract_sqrt_args(cursor, tail_rel)
    {
        let degree = deg_opt.map(|(d, d_rel)| {
            let mut parsed = parse_math_slice_indexed(d, cursor.base_offset + d_rel, false, cursor.depth + 1, cursor.scan);
            trim_edge_spaces(&mut parsed);
            parsed
        });
        let mut radicand = parse_math_slice_indexed(rad_raw, cursor.base_offset + rad_rel, false, cursor.depth + 1, cursor.scan);
        trim_edge_spaces(&mut radicand);
        return Some((ParsedStep::Atom { node: MathNode::Radical { degree, radicand }, is_roman_op: false }, consumed));
    }

    if name == "left"
        && let Some(matched) = cursor.scan.match_left_right(cursor.base_offset + cmd_rel, slice_end)
    {
        let body_rel_start = matched.body_start - cursor.base_offset;
        let body_rel_end = matched.body_end - cursor.base_offset;
        let inner_slice = cursor.slice.get(body_rel_start..body_rel_end)?;
        let mut inner = parse_math_slice_indexed(inner_slice, matched.body_start, false, cursor.depth + 1, cursor.scan);
        trim_edge_spaces(&mut inner);
        let consumed = (matched.total_end - cursor.base_offset).saturating_sub(tail_rel);
        return Some((
            ParsedStep::Atom {
                node: MathNode::Delimited { left: matched.left_delim, inner, right: matched.right_delim, explicit: true },
                is_roman_op: false,
            },
            consumed,
        ));
    }

    if matches!(name, "textbf" | "mathbf" | "textit" | "mathit" | "text" | "mathrm" | "textrm" | "operatorname" | "mbox")
        && let Some((inner, inner_rel, consumed)) = cursor.scan.match_braced_opt_ws(cursor.slice, tail_rel, cursor.base_offset)
    {
        let inner_text_mode = !matches!(name, "textit" | "mathit");
        let mut inner_nodes = parse_math_slice_indexed(inner, cursor.base_offset + inner_rel, inner_text_mode, cursor.depth + 1, cursor.scan);
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
    let mut stack: Vec<&mut MathNode> = nodes.iter_mut().collect();
    while let Some(node) = stack.pop() {
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
            MathNode::Group(inner) => {
                stack.extend(inner.iter_mut());
            }
            MathNode::Scripted { nucleus, sup, sub } => {
                stack.push(nucleus.as_mut());
                if let Some(sp) = sup {
                    stack.extend(sp.iter_mut());
                }
                if let Some(sb) = sub {
                    stack.extend(sb.iter_mut());
                }
            }
            _ => {}
        }
    }
}

fn try_parse_html_step(cursor: ParseCursor<'_>, rel_pos: usize, state: &mut SliceState) -> Option<(MathNode, usize)> {
    let rest = cursor.slice.get(rel_pos..)?;
    // Bound tag-head scan to at most 9 bytes (`close <= 8`) so bare `<<<<...` never rescans to EOF (`GAP-01-02`).
    let head = rest.as_bytes().get(..rest.len().min(9))?;
    let close = head.iter().position(|&b| b == b'>')?;
    if close == 0 || close > 8 {
        return None;
    }
    let tag = rest.get(1..close)?;
    let (tag_idx, end_tag, script_kind) = if tag.eq_ignore_ascii_case("b") {
        (0, "</b>", None)
    } else if tag.eq_ignore_ascii_case("i") {
        (1, "</i>", None)
    } else if tag.eq_ignore_ascii_case("sup") {
        (2, "</sup>", Some(SpanScript::Superscript))
    } else if tag.eq_ignore_ascii_case("sub") {
        (3, "</sub>", Some(SpanScript::Subscript))
    } else {
        return None;
    };
    let after_open_rel = rel_pos + close + 1;
    let close_pos = state.find_html_close(cursor.slice, tag_idx, after_open_rel, end_tag)?;
    let inner = cursor.slice.get(after_open_rel..after_open_rel + close_pos)?;
    let total = close + 1 + close_pos + end_tag.len();
    let mut inner_nodes = parse_math_slice_indexed(inner, cursor.base_offset + after_open_rel, cursor.text_mode, cursor.depth + 1, cursor.scan);
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
    let hb = haystack.as_bytes();
    let nb = needle.as_bytes();
    if nb.is_empty() || hb.len() < nb.len() {
        return None;
    }
    hb.windows(nb.len()).position(|w| w.eq_ignore_ascii_case(nb))
}

fn attach_scripts_if_any(mut nucleus: MathNode, cursor: ParseCursor<'_>, pos: &mut usize) -> MathNode {
    if cursor.depth >= MAX_TEX_DEPTH {
        return nucleus;
    }
    let mut sup: Option<Vec<MathNode>> = None;
    let mut sub: Option<Vec<MathNode>> = None;
    let mut script_depth = cursor.depth;

    while let Some(rest) = cursor.slice.get(*pos..) {
        let Some(marker) = rest.chars().next() else {
            break;
        };
        if marker != '^' && marker != '_' {
            break;
        }
        // Bound chained `Scripted` wrapping to `MAX_TEX_DEPTH` (`GAP-01-01`).
        let would_wrap = (marker == '^' && sup.is_some()) || (marker == '_' && sub.is_some());
        if would_wrap && script_depth + 1 >= MAX_TEX_DEPTH {
            break;
        }
        let arg_depth = script_depth + usize::from(would_wrap) + 1;
        let arg_cursor = ParseCursor { depth: arg_depth, ..cursor };
        let Some((arg_nodes, arg_consumed)) = parse_script_arg(arg_cursor, *pos + 1) else {
            break;
        };
        *pos += 1 + arg_consumed;
        if marker == '^' {
            if sup.is_some() {
                nucleus = MathNode::Scripted { nucleus: Box::new(nucleus), sup: sup.take(), sub: sub.take() };
                script_depth += 1;
            }
            sup = Some(arg_nodes);
        } else {
            if sub.is_some() {
                nucleus = MathNode::Scripted { nucleus: Box::new(nucleus), sup: sup.take(), sub: sub.take() };
                script_depth += 1;
            }
            sub = Some(arg_nodes);
        }
    }

    if sup.is_some() || sub.is_some() { MathNode::Scripted { nucleus: Box::new(nucleus), sup, sub } } else { nucleus }
}

fn parse_script_arg(cursor: ParseCursor<'_>, rel_pos: usize) -> Option<(Vec<MathNode>, usize)> {
    let s = cursor.slice.get(rel_pos..)?;
    let trimmed = s.trim_start_matches([' ', '\t']);
    let ws = s.len() - trimmed.len();
    let first = trimmed.chars().next()?;
    if first == '}' {
        return None;
    }
    let start_rel = rel_pos + ws;
    if first == '{' {
        let (inner, consumed) = cursor.scan.match_brace(cursor.slice, start_rel, cursor.base_offset)?;
        let mut nodes = parse_math_slice_indexed(inner, cursor.base_offset + start_rel + 1, cursor.text_mode, cursor.depth + 1, cursor.scan);
        trim_edge_spaces(&mut nodes);
        return Some((nodes, ws + consumed));
    }
    if first == '\\' {
        let mut local_pos = start_rel;
        let bs_cursor = ParseCursor { depth: cursor.depth + 1, ..cursor };
        let step = parse_backslash_step(bs_cursor, &mut local_pos);
        let node = match step {
            ParsedStep::Atom { node, .. } | ParsedStep::Raw(node) => node,
        };
        return Some((vec![node], local_pos.saturating_sub(rel_pos)));
    }
    let mut local_pos = start_rel;
    let step = parse_literal_atom(cursor.slice, &mut local_pos, cursor.text_mode);
    let node = match step {
        ParsedStep::Atom { node, .. } | ParsedStep::Raw(node) => node,
    };
    Some((vec![node], local_pos.saturating_sub(rel_pos)))
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

fn extract_one_math_arg<'a>(cursor: ParseCursor<'a>, rel_pos: usize) -> Option<((&'a str, usize), usize)> {
    let s = cursor.slice.get(rel_pos..)?;
    let trimmed = s.trim_start_matches([' ', '\t']);
    let ws = s.len() - trimmed.len();
    let start_rel = rel_pos + ws;
    if trimmed.starts_with('{') {
        let (inner, consumed) = cursor.scan.match_brace(cursor.slice, start_rel, cursor.base_offset)?;
        return Some(((inner, start_rel + 1), ws + consumed));
    }
    let ch = trimmed.chars().next()?;
    if ch == '\\' {
        let after = trimmed.get(1..)?;
        let nlen = after.bytes().take_while(u8::is_ascii_alphabetic).count();
        let end = if nlen > 0 { 1 + nlen } else { 1 + after.chars().next().map_or(0, char::len_utf8) };
        return Some(((trimmed.get(..end)?, start_rel), ws + end));
    }
    Some(((trimmed.get(..ch.len_utf8())?, start_rel), ws + ch.len_utf8()))
}

type TwoMathArgs<'a> = ((&'a str, usize), (&'a str, usize), usize);

fn extract_two_math_args<'a>(cursor: ParseCursor<'a>, rel_pos: usize) -> Option<TwoMathArgs<'a>> {
    let (a, ca) = extract_one_math_arg(cursor, rel_pos)?;
    let (b, cb) = extract_one_math_arg(cursor, rel_pos + ca)?;
    Some((a, b, ca + cb))
}

type SqrtArgs<'a> = (Option<(&'a str, usize)>, (&'a str, usize), usize);

fn extract_sqrt_args<'a>(cursor: ParseCursor<'a>, rel_pos: usize) -> Option<SqrtArgs<'a>> {
    let s = cursor.slice.get(rel_pos..)?;
    let trimmed = s.trim_start_matches([' ', '\t']);
    let ws = s.len() - trimmed.len();
    let start_rel = rel_pos + ws;
    let slice_end = cursor.base_offset + cursor.slice.len();
    if trimmed.starts_with('[')
        && let Some(close_abs) = cursor.scan.match_sqrt_bracket(cursor.base_offset + start_rel, slice_end)
    {
        let close_rel = close_abs - cursor.base_offset;
        let deg_start = start_rel + 1;
        let deg = cursor.slice.get(deg_start..close_rel)?;
        let after_deg_rel = close_rel + 1;
        let (rad, rad_consumed) = extract_one_math_arg(cursor, after_deg_rel)?;
        return Some((Some((deg, deg_start)), rad, (after_deg_rel + rad_consumed).saturating_sub(rel_pos)));
    }
    let (rad, rad_consumed) = extract_one_math_arg(cursor, rel_pos)?;
    Some((None, rad, rad_consumed))
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
