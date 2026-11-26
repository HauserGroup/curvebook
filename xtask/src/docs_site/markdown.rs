//! Markdown-to-HTML compiler backed by `pulldown-cmark` with script-free sanitization,
//! GitHub-style heading anchors + `{#custom-id}`, GFM callouts, `<figure>` promotion,
//! and strict-CSP static TeX-math rendering (`R12.4` / `R12.5`).

use std::collections::{BTreeMap, BTreeSet};

use pulldown_cmark::{BlockQuoteKind, CodeBlockKind, Event, HeadingLevel, LinkType, Options, Parser, Tag, TagEnd};

use crate::scan_util::{extract_html_tag_name, find_html_comment_end, find_html_tag_close, is_html_tag_start};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TocHeading {
    pub level: u8,
    pub id: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedMarkdown {
    pub title: Option<String>,
    pub headings: Vec<TocHeading>,
    pub html: String,
}

#[cfg(test)]
impl RenderedMarkdown {
    #[must_use]
    pub fn split_leading_h1(&self) -> (&str, Option<&str>, &str) {
        extract_leading_h1(&self.html)
    }
}

#[must_use]
pub fn extract_leading_h1(html: &str) -> (&str, Option<&str>, &str) {
    let (mut offset, mut depth, mut h1_start) = (0usize, 0usize, None);
    while let Some(rel_lt) = html.get(offset..).and_then(|r| r.find('<')) {
        let tag_abs = offset + rel_lt;
        let Some(after_lt) = html.get(tag_abs + 1..) else { break };
        if !is_html_tag_start(after_lt) {
            offset = tag_abs + 1;
            continue;
        }
        if let Some(after_c) = after_lt.strip_prefix("!--") {
            let Some(end) = find_html_comment_end(after_c) else { break };
            offset = tag_abs + 4 + end;
            continue;
        }
        let Some(tag_end) = find_html_tag_close(after_lt) else { break };
        let (after_tag, trimmed) = (tag_abs + 2 + tag_end, after_lt.get(..tag_end).unwrap_or("").trim());
        let (is_close, tag_lower) = (trimmed.starts_with('/'), extract_html_tag_name(trimmed).to_ascii_lowercase());
        if let Some(start) = h1_start {
            if is_close && tag_lower == "h1" {
                let end = after_tag + usize::from(html.get(after_tag..).is_some_and(|s| s.starts_with('\n')));
                return (html.get(..start).unwrap_or(""), Some(html.get(start..end).unwrap_or("")), html.get(end..).unwrap_or(""));
            }
        } else if matches!(tag_lower.as_str(), "blockquote" | "div" | "pre" | "code" | "ul" | "ol" | "table") {
            depth = if is_close { depth.saturating_sub(1) } else { depth + usize::from(!trimmed.ends_with('/')) };
        } else if !is_close && !trimmed.ends_with('/') && tag_lower == "h1" && depth == 0 {
            h1_start = Some(tag_abs);
        }
        offset = after_tag;
    }
    ("", None, html)
}

pub trait LinkResolver {
    fn resolve_href(&self, raw_href: &str) -> String;
    fn resolve_image(&self, raw_src: &str) -> String;
}

pub struct IdentityResolver;
impl LinkResolver for IdentityResolver {
    fn resolve_href(&self, raw_href: &str) -> String {
        raw_href.to_owned()
    }
    fn resolve_image(&self, raw_src: &str) -> String {
        raw_src.to_owned()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MathKind {
    DisplayBlock,
    InlineBlockSpan,
    Inline,
}

struct MathSlot {
    kind: MathKind,
    tex: String,
    raw: String,
}

const MATH_CLOSE_SENTINEL: char = '\u{E001}';
const MAX_MATH_DEPTH: usize = 32;
const MAX_SQRT_DEGREE_BYTES: usize = 128;

pub fn render_markdown(input: &str, resolver: &dyn LinkResolver) -> RenderedMarkdown {
    let normalized = input.replace("\r\n", "\n");
    let sentinel_open = choose_math_sentinel(&normalized);
    let (protected, math_slots) = protect_math_spans(&normalized, &sentinel_open);
    let opts =
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_GFM | Options::ENABLE_HEADING_ATTRIBUTES | Options::ENABLE_TASKLISTS;
    let mut r = HtmlRenderer {
        resolver,
        sentinel_open: &sentinel_open,
        math_slots: &math_slots,
        used_ids: ["main", "page-title"].into_iter().map(String::from).collect(),
        headings: Vec::new(),
        title: None,
        out: String::with_capacity(normalized.len().saturating_mul(2)),
        blockquote_ends: Vec::new(),
        in_table_head: false,
        table_tbody_open: false,
        in_code_block: false,
    };
    r.render_stream(Parser::new_ext(&protected, opts));
    RenderedMarkdown { title: r.title, headings: r.headings, html: r.out }
}

fn choose_math_sentinel(input: &str) -> String {
    const BASE: &str = "\u{E000}CBM";
    if !input.contains(BASE) {
        return String::from("\u{E000}CBM:");
    }
    let (mut max_xs, mut rest) = (0usize, input);
    while let Some((_, after)) = rest.split_once(BASE) {
        max_xs = max_xs.max(after.bytes().take_while(|&b| b == b'X').count() + 1);
        rest = after;
    }
    format!("{BASE}{}:", "X".repeat(max_xs))
}

struct HtmlRenderer<'a> {
    resolver: &'a dyn LinkResolver,
    sentinel_open: &'a str,
    math_slots: &'a [MathSlot],
    used_ids: BTreeSet<String>,
    headings: Vec<TocHeading>,
    title: Option<String>,
    out: String,
    blockquote_ends: Vec<&'static str>,
    in_table_head: bool,
    table_tbody_open: bool,
    in_code_block: bool,
}

impl HtmlRenderer<'_> {
    fn unique_id(&mut self, base: &str) -> String {
        let norm = if base.is_empty() { String::from("section") } else { base.to_owned() };
        if self.used_ids.insert(norm.clone()) {
            return norm;
        }
        (1usize..).map(|n| format!("{norm}-{n}")).find(|c| self.used_ids.insert(c.clone())).unwrap_or(norm)
    }

    fn restore(&self, s: &str) -> String {
        restore_math_sentinels(s, self.sentinel_open, self.math_slots)
    }

    fn render_stream<'e>(&mut self, mut parser: impl Iterator<Item = Event<'e>>) {
        while let Some(ev) = parser.next() {
            match ev {
                Event::Start(Tag::Paragraph) => {
                    let inner = collect_until_end(&mut parser, |t| matches!(t, TagEnd::Paragraph));
                    self.flush_paragraph(inner);
                }
                Event::Start(Tag::Heading { level, id, .. }) => {
                    let inner = collect_until_end(&mut parser, |t| matches!(t, TagEnd::Heading(_)));
                    let explicit = id.map(|c| self.restore(&c));
                    self.flush_heading(level, explicit.as_deref(), inner);
                }
                Event::Start(Tag::Image { dest_url, title, .. }) => self.emit_inline_image(&dest_url, &title, &mut parser),
                Event::Start(tag) => self.start_tag(tag),
                Event::End(tag) => self.end_tag(tag),
                other => self.emit_leaf_event(other, false),
            }
        }
    }

    fn push_img_tag(&mut self, dest_url: &str, alt: &str, title: &str) {
        let resolved = sanitize_resolved_url(&self.resolver.resolve_image(&self.restore(dest_url)));
        self.out.push_str("<img src=\"");
        push_html_escaped(&resolved, &mut self.out);
        self.out.push_str("\" alt=\"");
        push_html_escaped(alt, &mut self.out);
        self.out.push('"');
        if !title.is_empty() {
            self.out.push_str(" title=\"");
            push_html_escaped(&self.restore(title), &mut self.out);
            self.out.push('"');
        }
        self.out.push_str(" loading=\"lazy\">");
    }

    fn emit_inline_image<'e>(&mut self, dest_url: &str, title: &str, iter: &mut impl Iterator<Item = Event<'e>>) {
        let alt = events_to_plain_text(&collect_until_end(iter, |t| matches!(t, TagEnd::Image)), self.sentinel_open, self.math_slots);
        self.push_img_tag(dest_url, &alt, title);
    }

    fn flush_paragraph(&mut self, events: Vec<Event<'_>>) {
        if let Some(slot_id) = standalone_display_math_slot(&events, self.sentinel_open)
            && let Some(slot) = self.math_slots.get(slot_id).filter(|s| s.kind == MathKind::DisplayBlock)
        {
            return push_math_span(&mut self.out, MathKind::DisplayBlock, &slot.tex, true);
        }
        if let Some((dest_url, alt)) = standalone_figure_image(&events, self.sentinel_open, self.math_slots) {
            self.out.push_str("<figure class=\"doc-figure\">");
            self.push_img_tag(&dest_url, &alt, "");
            if !alt.trim().is_empty() {
                self.out.push_str("<figcaption>");
                push_html_escaped(alt.trim(), &mut self.out);
                self.out.push_str("</figcaption>");
            }
            self.out.push_str("</figure>\n");
            return;
        }
        self.out.push_str("<p>");
        self.render_inline_events(events);
        self.out.push_str("</p>\n");
    }

    fn flush_heading(&mut self, level: HeadingLevel, explicit_id: Option<&str>, events: Vec<Event<'_>>) {
        let plain = events_to_plain_text(&events, self.sentinel_open, self.math_slots);
        let lvl = match level {
            HeadingLevel::H1 => 1,
            HeadingLevel::H2 => 2,
            HeadingLevel::H3 => 3,
            HeadingLevel::H4 => 4,
            HeadingLevel::H5 => 5,
            HeadingLevel::H6 => 6,
        };
        if lvl == 1 && self.title.is_none() && self.blockquote_ends.is_empty() {
            self.title = Some(plain.clone());
        }
        let id = self.unique_id(explicit_id.filter(|s| !s.is_empty()).map_or_else(|| github_slug(&plain), str::to_owned).as_str());
        self.headings.push(TocHeading { level: lvl, id: id.clone(), text: plain });
        self.out.push_str(&format!("<h{lvl} id=\""));
        push_html_escaped(&id, &mut self.out);
        self.out.push_str("\" tabindex=\"-1\">");
        self.render_inline_events(events);
        self.out.push_str(&format!("</h{lvl}>\n"));
    }

    fn render_inline_events(&mut self, events: Vec<Event<'_>>) {
        let mut iter = events.into_iter();
        while let Some(ev) = iter.next() {
            match ev {
                Event::Start(Tag::Image { dest_url, title, .. }) => self.emit_inline_image(&dest_url, &title, &mut iter),
                Event::Start(tag) => self.start_tag(tag),
                Event::End(tag) => self.end_tag(tag),
                other => self.emit_leaf_event(other, true),
            }
        }
    }

    fn start_tag(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::BlockQuote(Some(alert)) => {
                let (slug, title) = match alert {
                    BlockQuoteKind::Note => ("note", "Note"),
                    BlockQuoteKind::Tip => ("tip", "Tip"),
                    BlockQuoteKind::Important => ("important", "Important"),
                    BlockQuoteKind::Warning => ("warning", "Warning"),
                    BlockQuoteKind::Caution => ("caution", "Caution"),
                };
                self.out.push_str(&format!("<div class=\"callout callout-{slug}\" role=\"note\">\n<p class=\"callout-title\">{title}</p>\n"));
                self.blockquote_ends.push("</div>\n");
            }
            Tag::BlockQuote(None) => (self.out.push_str("<blockquote>\n"), self.blockquote_ends.push("</blockquote>\n")).0,
            Tag::CodeBlock(kind) => {
                self.in_code_block = true;
                let lang = match kind {
                    CodeBlockKind::Fenced(info) => self.restore(&info).split_whitespace().next().unwrap_or("").to_owned(),
                    CodeBlockKind::Indented => String::new(),
                };
                if lang.is_empty() {
                    self.out.push_str("<pre><code>");
                } else {
                    self.out.push_str("<pre><code class=\"language-");
                    push_html_escaped(&lang, &mut self.out);
                    self.out.push_str("\">");
                }
            }
            Tag::List(Some(start)) if start != 1 => self.out.push_str(&format!("<ol start=\"{start}\">\n")),
            Tag::Table(_) => (self.table_tbody_open, ()) = (false, self.out.push_str("<div class=\"table-wrap\"><table>\n")),
            Tag::TableHead => (self.in_table_head, ()) = (true, self.out.push_str("<thead><tr>")),
            Tag::TableRow => {
                if !self.table_tbody_open {
                    (self.table_tbody_open, ()) = (true, self.out.push_str("<tbody>\n"));
                }
                self.out.push_str("<tr>");
            }
            Tag::Link { link_type, dest_url, title, .. } => {
                let d = self.restore(&dest_url);
                let raw_href = if link_type == LinkType::Email && !d.starts_with("mailto:") { format!("mailto:{d}") } else { d };
                self.out.push_str("<a href=\"");
                push_html_escaped(&sanitize_resolved_url(&self.resolver.resolve_href(&raw_href)), &mut self.out);
                self.out.push('"');
                if !title.is_empty() {
                    self.out.push_str(" title=\"");
                    push_html_escaped(&self.restore(&title), &mut self.out);
                    self.out.push('"');
                }
                self.out.push('>');
            }
            other => self.out.push_str(match other {
                Tag::List(None) => "<ul>\n",
                Tag::List(Some(_)) => "<ol>\n",
                Tag::Item => "<li>",
                Tag::TableCell => {
                    if self.in_table_head {
                        "<th>"
                    } else {
                        "<td>"
                    }
                }
                Tag::Emphasis => "<em>",
                Tag::Strong => "<strong>",
                Tag::Strikethrough => "<del>",
                Tag::Superscript => "<sup>",
                Tag::Subscript => "<sub>",
                _ => "",
            }),
        }
    }

    fn end_tag(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::BlockQuote(_) => {
                if let Some(c) = self.blockquote_ends.pop() {
                    self.out.push_str(c);
                }
            }
            TagEnd::CodeBlock => (self.in_code_block, ()) = (false, self.out.push_str("</code></pre>\n")),
            TagEnd::TableHead => (self.in_table_head, ()) = (false, self.out.push_str("</tr></thead>\n")),
            TagEnd::Table => {
                if self.table_tbody_open {
                    (self.table_tbody_open, ()) = (false, self.out.push_str("</tbody>\n"));
                }
                self.out.push_str("</table></div>\n");
            }
            other => self.out.push_str(match other {
                TagEnd::List(true) => "</ol>\n",
                TagEnd::List(false) => "</ul>\n",
                TagEnd::Item => "</li>\n",
                TagEnd::TableRow => "</tr>\n",
                TagEnd::TableCell => {
                    if self.in_table_head {
                        "</th>"
                    } else {
                        "</td>"
                    }
                }
                TagEnd::Emphasis => "</em>",
                TagEnd::Strong => "</strong>",
                TagEnd::Strikethrough => "</del>",
                TagEnd::Link => "</a>",
                TagEnd::Superscript => "</sup>",
                TagEnd::Subscript => "</sub>",
                _ => "",
            }),
        }
    }

    fn emit_leaf_event(&mut self, event: Event<'_>, inline_context: bool) {
        match event {
            Event::Text(text) if self.in_code_block => push_html_escaped(&self.restore(&text), &mut self.out),
            Event::Text(text) => visit_math_sentinels(&text, self.sentinel_open, self.math_slots, &mut self.out, push_html_escaped, |slot, out| {
                push_math_span(out, slot.kind, &slot.tex, false);
            }),
            Event::Code(code) => {
                self.out.push_str("<code>");
                push_html_escaped(&self.restore(&code), &mut self.out);
                self.out.push_str("</code>");
            }
            Event::InlineMath(tex) => push_math_span(&mut self.out, MathKind::Inline, &tex, false),
            Event::DisplayMath(tex) => push_math_span(&mut self.out, MathKind::DisplayBlock, &tex, true),
            Event::SoftBreak => self.out.push(if inline_context { ' ' } else { '\n' }),
            Event::HardBreak => self.out.push_str("<br>\n"),
            Event::Rule => self.out.push_str("<hr>\n"),
            Event::Html(raw) | Event::InlineHtml(raw) => {
                let r = self.restore(&raw);
                let t = r.trim();
                if t.is_empty() || (t.starts_with("<!--") && t.ends_with("-->")) {
                    return;
                }
                if matches!(t.to_ascii_lowercase().as_str(), "<br>" | "<br/>" | "<br />") {
                    self.out.push_str("<br>");
                } else {
                    push_html_escaped(&r, &mut self.out);
                }
            }
            Event::TaskListMarker(checked) => {
                self.out.push_str(if checked { "<input type=\"checkbox\" disabled checked> " } else { "<input type=\"checkbox\" disabled> " })
            }
            Event::FootnoteReference(name) => {
                self.out.push_str("<sup>[");
                push_html_escaped(&self.restore(&name), &mut self.out);
                self.out.push_str("]</sup>");
            }
            Event::Start(_) | Event::End(_) => {}
        }
    }
}

fn push_math_span(out: &mut String, kind: MathKind, tex: &str, trailing_nl: bool) {
    let (open, close) = match kind {
        MathKind::DisplayBlock => ("<div class=\"math-block\"><span class=\"math\">", if trailing_nl { "</span></div>\n" } else { "</span></div>" }),
        MathKind::InlineBlockSpan => ("<span class=\"math math-block\">", "</span>"),
        MathKind::Inline => ("<span class=\"math\">", "</span>"),
    };
    out.push_str(open);
    render_tex_math(tex.trim(), 0, false, out);
    out.push_str(close);
}

fn collect_until_end<'e>(iter: &mut impl Iterator<Item = Event<'e>>, is_end: impl Fn(&TagEnd) -> bool) -> Vec<Event<'e>> {
    let (mut events, mut depth) = (Vec::new(), 1usize);
    for ev in iter {
        match &ev {
            Event::Start(_) => depth += 1,
            Event::End(t) => {
                depth -= 1;
                if depth == 0 && is_end(t) {
                    break;
                }
            }
            _ => {}
        }
        events.push(ev);
    }
    events
}

fn visit_math_sentinels(
    text: &str,
    sentinel_open: &str,
    slots: &[MathSlot],
    out: &mut String,
    mut on_text: impl FnMut(&str, &mut String),
    mut on_slot: impl FnMut(&MathSlot, &mut String),
) {
    let mut rest = text;
    while let Some((before, after_open)) = rest.split_once(sentinel_open) {
        on_text(before, out);
        if let Some((digits, tail)) = after_open.split_once(MATH_CLOSE_SENTINEL)
            && let Some(slot) = digits.parse::<usize>().ok().and_then(|i| slots.get(i))
        {
            on_slot(slot, out);
            rest = tail;
            continue;
        }
        on_text(sentinel_open, out);
        rest = after_open;
    }
    on_text(rest, out);
}

fn restore_math_sentinels(text: &str, sentinel_open: &str, slots: &[MathSlot]) -> String {
    if !text.contains(sentinel_open) {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    visit_math_sentinels(text, sentinel_open, slots, &mut out, |s, o| o.push_str(s), |slot, o| o.push_str(&slot.raw));
    out
}

fn standalone_display_math_slot(events: &[Event<'_>], sentinel_open: &str) -> Option<usize> {
    let [Event::Text(text)] = events else { return None };
    let (digits, after) = text.trim().strip_prefix(sentinel_open)?.split_once(MATH_CLOSE_SENTINEL)?;
    after.is_empty().then(|| digits.parse().ok()).flatten()
}

fn standalone_figure_image(events: &[Event<'_>], sentinel_open: &str, slots: &[MathSlot]) -> Option<(String, String)> {
    let (Some(Event::Start(Tag::Image { dest_url, .. })), Some(Event::End(TagEnd::Image))) = (events.first(), events.last()) else { return None };
    let inner = events.get(1..events.len().saturating_sub(1))?;
    let mut depth = 1usize;
    for ev in inner {
        match ev {
            Event::Start(_) => depth += 1,
            Event::End(_) => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return None;
                }
            }
            _ => {}
        }
    }
    Some((dest_url.to_string(), events_to_plain_text(inner, sentinel_open, slots)))
}

fn events_to_plain_text(events: &[Event<'_>], sentinel_open: &str, slots: &[MathSlot]) -> String {
    let mut out = String::new();
    for ev in events {
        match ev {
            Event::Text(t) => {
                visit_math_sentinels(t, sentinel_open, slots, &mut out, |s, o| o.push_str(s), |slot, o| render_tex_math(slot.tex.trim(), 0, true, o))
            }
            Event::Code(c) => out.push_str(&restore_math_sentinels(c, sentinel_open, slots)),
            Event::InlineMath(m) | Event::DisplayMath(m) => render_tex_math(m.trim(), 0, true, &mut out),
            Event::SoftBreak | Event::HardBreak => out.push(' '),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn sanitize_resolved_url(url: &str) -> String {
    let cleaned: String = url.trim().chars().filter(|c| !c.is_ascii_control()).collect();
    let norm: String = cleaned.chars().filter(|c| !c.is_ascii_whitespace()).map(|c| c.to_ascii_lowercase()).collect();
    if norm.starts_with("javascript:") || norm.starts_with("vbscript:") || norm.starts_with("data:") { String::from("#") } else { cleaned }
}

#[must_use]
pub fn github_slug(text: &str) -> String {
    text.chars()
        .filter_map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '_' | '-') { Some(c.to_ascii_lowercase()) } else { matches!(c, ' ' | '\t').then_some('-') }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_owned()
}

pub(super) fn parse_fence_open(trimmed: &str) -> Option<(u8, usize)> {
    let first = *trimmed.as_bytes().first().filter(|&&b| b == b'`' || b == b'~')?;
    let count = trimmed.bytes().take_while(|&b| b == first).count();
    (count >= 3 && !(first == b'`' && trimmed.get(count..).is_some_and(|i| i.contains('`')))).then_some((first, count))
}

fn leading_indent_cols(line: &str) -> usize {
    line.bytes().take_while(|&b| b == b' ' || b == b'\t').map(|b| if b == b'\t' { 4 } else { 1 }).sum()
}

fn strip_blockquote_prefix(mut rest: &str) -> (usize, &str) {
    let mut depth = 0usize;
    while let Some(after_gt) =
        (rest.bytes().take_while(|&b| b == b' ').count() <= 3).then(|| rest.trim_start_matches(' ').strip_prefix('>')).flatten()
    {
        depth += 1;
        rest = after_gt.strip_prefix(' ').unwrap_or(after_gt);
    }
    (depth, rest)
}

fn protect_math_spans(input: &str, sentinel_open: &str) -> (String, Vec<MathSlot>) {
    let lines: Vec<&str> = input.lines().collect();
    let (mut out, mut slots, mut fence, mut no_close_until, mut idx) =
        (String::with_capacity(input.len() + 64), Vec::new(), None::<(usize, u8, usize)>, 0usize, 0usize);
    while let Some(&line) = lines.get(idx) {
        let (q_depth, inner) = strip_blockquote_prefix(line);
        if let Some((fq, ch, min_len)) = fence {
            if q_depth < fq {
                fence = None;
            } else {
                out.push_str(line);
                out.push('\n');
                let t = inner.trim();
                let n = t.bytes().take_while(|&b| b == ch).count();
                if q_depth == fq && n >= min_len && t.get(n..).is_some_and(|r| r.trim().is_empty()) {
                    fence = None;
                }
                idx += 1;
                continue;
            }
        }
        if leading_indent_cols(inner) < 4
            && let Some((ch, len)) = parse_fence_open(inner.trim())
        {
            fence = Some((q_depth, ch, len));
            out.push_str(line);
            out.push('\n');
            idx += 1;
            continue;
        }
        if leading_indent_cols(line) < 4
            && let Some((tex, next_idx)) = extract_display_math_block(&lines, idx, &mut no_close_until)
        {
            let id = slots.len();
            slots.push(MathSlot { kind: MathKind::DisplayBlock, tex, raw: lines.get(idx..next_idx).map(|s| s.join("\n")).unwrap_or_default() });
            out.push_str(&format!("\n{sentinel_open}{id}{MATH_CLOSE_SENTINEL}\n\n"));
            idx = next_idx;
            continue;
        }
        protect_line_math(line, sentinel_open, &mut slots, &mut out);
        out.push('\n');
        idx += 1;
    }
    (out, slots)
}

fn extract_display_math_block(lines: &[&str], idx: usize, no_close_until: &mut usize) -> Option<(String, usize)> {
    let after_open = lines.get(idx)?.trim().strip_prefix("$$")?;
    if let Some(inner) = after_open.strip_suffix("$$") {
        let clean = inner.trim();
        return (!inner.contains("$$") && !clean.is_empty()).then(|| (clean.to_owned(), idx + 1));
    }
    if after_open.contains("$$") || idx + 1 < *no_close_until {
        return None;
    }
    let mut close_idx = None;
    for (off, line) in lines.get(idx + 1..).unwrap_or(&[]).iter().enumerate() {
        let t = line.trim();
        if parse_fence_open(t).is_some() {
            *no_close_until = (*no_close_until).max(idx + 1 + off);
            return None;
        }
        let escaped = t.strip_suffix("$$").is_some_and(|b| b.bytes().rev().take_while(|&c| c == b'\\').count() % 2 == 1);
        if t == "$$" || (t.ends_with("$$") && !escaped) {
            close_idx = Some(idx + 1 + off);
            break;
        }
    }
    let Some(ci) = close_idx else {
        *no_close_until = (*no_close_until).max(lines.len());
        return None;
    };
    let joined = std::iter::once(after_open.trim())
        .chain(lines.get(idx + 1..ci)?.iter().map(|l| l.trim()))
        .chain(lines.get(ci)?.trim().strip_suffix("$$").map(str::trim))
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    (!joined.trim().is_empty()).then(|| (joined.trim().to_owned(), ci + 1))
}

fn protect_line_math(line: &str, sentinel_open: &str, slots: &mut Vec<MathSlot>, out: &mut String) {
    let (b, mut last_ticks, mut j) = (line.as_bytes(), BTreeMap::new(), 0usize);
    while j < b.len() {
        if b[j] == b'`' {
            let s = j;
            while j < b.len() && b[j] == b'`' {
                j += 1;
            }
            last_ticks.insert(j - s, s);
        } else {
            j += 1;
        }
    }
    let mut i = 0usize;
    while let Some(rest) = line.get(i..) {
        let Some(ch) = rest.chars().next() else { break };
        if ch == '\\' {
            let extra = line.get(i + 1..).and_then(|s| s.chars().next()).map_or(0, char::len_utf8);
            out.push_str(rest.get(..1 + extra).unwrap_or("\\"));
            i += 1 + extra;
            continue;
        }
        if ch == '`' {
            let run = rest.bytes().take_while(|&b| b == b'`').count();
            let consumed = if last_ticks.get(&run).is_some_and(|&last| last > i) { parse_code_span_len(rest).unwrap_or(run) } else { run };
            out.push_str(rest.get(..consumed).unwrap_or(""));
            i += consumed;
            continue;
        }
        let math_hit = if rest.starts_with("$$") {
            parse_delimited_math(rest, i, &last_ticks, true).map(|(t, c)| (MathKind::InlineBlockSpan, t, c))
        } else if ch == '$' && !line.get(..i).and_then(|p| p.chars().next_back()).is_some_and(|c| c.is_ascii_alphanumeric()) {
            parse_delimited_math(rest, i, &last_ticks, false).map(|(t, c)| (MathKind::Inline, t, c))
        } else {
            None
        };
        if let Some((kind, tex, consumed)) = math_hit {
            let id = slots.len();
            slots.push(MathSlot { kind, tex: tex.to_owned(), raw: rest.get(..consumed).unwrap_or("").to_owned() });
            out.push_str(&format!("{sentinel_open}{id}{MATH_CLOSE_SENTINEL}"));
            i += consumed;
            continue;
        }
        out.push(ch);
        i += ch.len_utf8();
    }
}

pub(super) fn parse_code_span_len(slice: &str) -> Option<usize> {
    let ticks = slice.bytes().take_while(|&b| b == b'`').count();
    let (after, mut pos) = (slice.get(ticks..)?, 0usize);
    while let Some(cur) = after.get(pos..) {
        if cur.starts_with('`') {
            let run = cur.bytes().take_while(|&b| b == b'`').count();
            if run == ticks {
                return Some(ticks + pos + ticks);
            }
            pos += run;
        } else {
            pos += cur.chars().next()?.len_utf8();
        }
    }
    None
}

fn parse_delimited_math<'a>(rest: &'a str, line_offset: usize, last_ticks: &BTreeMap<usize, usize>, block: bool) -> Option<(&'a str, usize)> {
    let delim = if block { "$$" } else { "$" };
    let after = rest.strip_prefix(delim)?;
    if !block && (after.is_empty() || after.starts_with('$') || after.chars().next().is_some_and(char::is_whitespace)) {
        return None;
    }
    let (mut i, mut brace_depth) = (0usize, 0usize);
    while let Some(cur) = after.get(i..) {
        let ch = cur.chars().next()?;
        if ch == '\\' {
            i += 1 + after.get(i + 1..).and_then(|s| s.chars().next()).map_or(0, char::len_utf8);
            continue;
        }
        if ch == '`' {
            let run = cur.bytes().take_while(|&b| b == b'`').count();
            if last_ticks.get(&run).is_some_and(|&last| last > line_offset + delim.len() + i) && parse_code_span_len(cur).is_some() {
                return None;
            }
            i += run;
            continue;
        }
        match ch {
            '{' => {
                brace_depth = brace_depth.saturating_add(1);
                if brace_depth > MAX_MATH_DEPTH {
                    return None;
                }
            }
            '}' => brace_depth = brace_depth.checked_sub(1)?,
            _ if brace_depth == 0 && cur.starts_with(delim) => {
                let inner = after.get(..i)?;
                if block {
                    return (!inner.trim().is_empty()).then_some((inner, i + 4));
                }
                let after_close = after.get(i + 1..)?;
                return (!inner.is_empty()
                    && !inner.chars().last().is_some_and(char::is_whitespace)
                    && !after_close.starts_with('$')
                    && !after_close.chars().next().is_some_and(|c| c.is_ascii_alphanumeric() || c == '_'))
                .then_some((inner, i + 2));
            }
            _ => {}
        }
        i += ch.len_utf8();
    }
    None
}

#[must_use]
pub fn render_math_html(tex: &str) -> String {
    let mut out = String::with_capacity(tex.len().saturating_mul(2) + 32);
    push_math_span(&mut out, MathKind::Inline, tex, false);
    out
}

fn push_math_text(s: &str, plain: bool, out: &mut String) {
    if plain { out.push_str(s) } else { push_html_escaped(s, out) }
}
fn push_math_char(ch: char, plain: bool, out: &mut String) {
    if plain { out.push(ch) } else { push_char_escaped(ch, out) }
}
fn render_wrapped_tex(out: &mut String, open: &str, tex: &str, close: &str, depth: usize, plain: bool) {
    out.push_str(open);
    render_tex_math(tex, depth + 1, plain, out);
    out.push_str(close);
}

fn render_tex_math(tex: &str, depth: usize, plain: bool, out: &mut String) {
    if depth > MAX_MATH_DEPTH {
        return push_math_text(tex, plain, out);
    }
    let tex = if plain { tex } else { tex.trim() };
    let (start_len, mut i) = (out.len(), 0usize);
    while let Some(rest) = tex.get(i..) {
        let Some(ch) = rest.chars().next() else { break };
        if !plain && ch.is_whitespace() {
            while let Some(nc) = tex.get(i..).and_then(|s| s.chars().next()).filter(|c| c.is_whitespace()) {
                i += nc.len_utf8();
            }
            if out.len() > start_len && !out.ends_with(' ') {
                out.push(' ');
            }
            continue;
        }
        if ch == '{'
            && let Some((inner, consumed)) = extract_braced(rest)
        {
            render_tex_math(inner, depth + 1, plain, out);
            i += consumed;
        } else if (ch == '_' || ch == '^')
            && let Some((arg, consumed)) = extract_math_arg(rest.get(1..).unwrap_or(""), depth)
        {
            let (o, c) = if plain {
                ("", "")
            } else if ch == '_' {
                ("<sub>", "</sub>")
            } else {
                ("<sup>", "</sup>")
            };
            render_wrapped_tex(out, o, arg, c, depth, plain);
            i += 1 + consumed;
        } else if ch == '\\' {
            i += render_tex_command(rest, depth, plain, out);
        } else {
            push_math_char(ch, plain, out);
            i += ch.len_utf8();
        }
    }
}

fn render_tex_command(rest: &str, depth: usize, plain: bool, out: &mut String) -> usize {
    let Some(after) = rest.get(1..) else { return 1 };
    let Some(first) = after.chars().next() else { return 1 };
    if !first.is_ascii_alphabetic() {
        match first {
            '!' => {}
            ',' if !plain => out.push('\u{2009}'),
            ',' | ';' | ':' | ' ' | '\\' => out.push(' '),
            '|' => out.push('\u{2016}'),
            _ => push_math_char(first, plain, out),
        }
        return 1 + first.len_utf8();
    }
    let nlen = after.bytes().take_while(u8::is_ascii_alphabetic).count();
    let (name, tail) = (after.get(..nlen).unwrap_or(""), after.get(nlen..).unwrap_or(""));
    if matches!(name, "frac" | "dfrac" | "tfrac" | "binom")
        && let Some((num, den, c)) = extract_two_math_args(tail, depth)
    {
        let (o, m, e) = if name == "binom" {
            ("(", " choose ", ")")
        } else if plain {
            ("", "/", "")
        } else {
            ("<span class=\"math-frac\"><span class=\"math-num\">", "</span><span class=\"math-den\">", "</span></span>")
        };
        render_wrapped_tex(out, o, num.trim(), m, depth, plain);
        render_wrapped_tex(out, "", den.trim(), e, depth, plain);
        return 1 + nlen + c;
    }
    if name == "sqrt"
        && let Some((deg, rad, c)) = extract_sqrt_args(tail, depth)
    {
        if !plain && let Some(d) = deg {
            render_wrapped_tex(out, "<sup>", d.trim(), "</sup>", depth, false);
        }
        let (o, e) = if plain { ("\u{221A}", "") } else { ("\u{221A}<span class=\"math-rad\">", "</span>") };
        render_wrapped_tex(out, o, rad.trim(), e, depth, plain);
        return 1 + nlen + c;
    }
    if matches!(name, "text" | "mathrm" | "textrm" | "operatorname" | "textbf" | "mathbf" | "mbox" | "textit" | "mathit")
        && let Some((inner, c)) = extract_braced_opt_ws(tail)
    {
        let (o, e) = if plain {
            ("", "")
        } else if matches!(name, "textit" | "mathit") {
            ("<em>", "</em>")
        } else {
            ("<span class=\"math-text\">", "</span>")
        };
        out.push_str(o);
        render_tex_text_mode(inner, plain, out);
        out.push_str(e);
        return 1 + nlen + c;
    }
    if matches!(name, "bar" | "overline" | "hat" | "widehat" | "tilde" | "widetilde")
        && let Some((arg, c)) = extract_math_arg(tail, depth)
    {
        if plain {
            render_tex_math(arg.trim(), depth + 1, true, out);
        } else {
            let accent = match name {
                "bar" | "overline" => '\u{0304}',
                "hat" | "widehat" => '\u{0302}',
                _ => '\u{0303}',
            };
            let mut inner_html = String::new();
            render_tex_math(arg.trim(), depth + 1, false, &mut inner_html);
            push_accented_html(&inner_html, accent, out);
        }
        return 1 + nlen + c;
    }
    if matches!(name, "left" | "right") {
        return 1 + nlen + usize::from(tail.starts_with('.'));
    }
    if matches!(name, "quad" | "qquad") {
        out.push_str(if plain {
            " "
        } else if name == "quad" {
            "\u{2003}"
        } else {
            "\u{2003}\u{2003}"
        });
        return 1 + nlen;
    }
    if matches!(name, "TeX" | "LaTeX") || ROMAN_MATH_FUNCTIONS.split_ascii_whitespace().any(|w| w == name) {
        if plain {
            out.push_str(name);
        } else {
            out.push_str("<span class=\"math-text\">");
            out.push_str(name);
            out.push_str("</span>");
        }
        return 1 + nlen;
    }
    if let Some(sym) = lookup_tex_symbol(name) {
        let extra = usize::from(matches!(name, "textbackslash" | "textless" | "textgreater" | "textdegree") && tail.starts_with("{}")) * 2;
        push_math_text(sym, plain, out);
        return 1 + nlen + extra;
    }
    push_math_text(name, plain, out);
    1 + nlen
}

fn render_tex_text_mode(inner: &str, plain: bool, out: &mut String) {
    let mut i = 0usize;
    while let Some(rest) = inner.get(i..) {
        let Some(ch) = rest.chars().next() else { break };
        if ch == '\\'
            && let Some(next_ch) = rest.get(1..).and_then(|s| s.chars().next())
        {
            if next_ch.is_ascii_punctuation() || next_ch == ' ' {
                push_math_char(next_ch, plain, out);
                i += 1 + next_ch.len_utf8();
                continue;
            }
            let after = rest.get(1..).unwrap_or("");
            let nlen = after.bytes().take_while(u8::is_ascii_alphabetic).count();
            if nlen > 0
                && let Some(sym) = lookup_tex_symbol(after.get(..nlen).unwrap_or(""))
            {
                push_math_text(sym, plain, out);
                i += 1 + nlen;
                continue;
            }
        }
        push_math_char(ch, plain, out);
        i += ch.len_utf8();
    }
}

fn push_accented_html(rendered: &str, accent: char, out: &mut String) {
    let (mut inserted, mut in_tag, mut in_entity) = (false, false, false);
    for ch in rendered.chars() {
        out.push(ch);
        if inserted {
            continue;
        }
        match ch {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            '&' if !in_tag => in_entity = true,
            ';' if in_entity => (in_entity, inserted, ()) = (false, true, out.push(accent)),
            _ if !in_tag && !in_entity && !ch.is_whitespace() => (inserted, ()) = (true, out.push(accent)),
            _ => {}
        }
    }
    if !inserted {
        out.push(accent);
    }
}

fn extract_braced(s: &str) -> Option<(&str, usize)> {
    if !s.starts_with('{') {
        return None;
    }
    let (mut depth, mut escaped) = (0usize, false);
    for (idx, ch) in s.char_indices() {
        if escaped {
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == '{' {
            depth = depth.saturating_add(1);
            if depth > MAX_MATH_DEPTH {
                return None;
            }
        } else if ch == '}' {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return Some((s.get(1..idx)?, idx + 1));
            }
        }
    }
    None
}

fn extract_braced_opt_ws(s: &str) -> Option<(&str, usize)> {
    let trimmed = s.trim_start_matches([' ', '\t']);
    let (inner, consumed) = extract_braced(trimmed)?;
    Some((inner, (s.len() - trimmed.len()) + consumed))
}

fn extract_math_arg(s: &str, depth: usize) -> Option<(&str, usize)> {
    if depth > MAX_MATH_DEPTH {
        return None;
    }
    let trimmed = s.trim_start_matches([' ', '\t']);
    let ws = s.len() - trimmed.len();
    if trimmed.starts_with('{') {
        let (inner, c) = extract_braced(trimmed)?;
        return Some((inner, ws + c));
    }
    let ch = trimmed.chars().next()?;
    if ch == '\\' {
        let after = trimmed.get(1..)?;
        let nlen = after.bytes().take_while(u8::is_ascii_alphabetic).count();
        let mut end = if nlen > 0 { 1 + nlen } else { 1 + after.chars().next().map_or(0, char::len_utf8) };
        if nlen > 0 {
            let (name, tail) = (after.get(..nlen).unwrap_or(""), after.get(nlen..).unwrap_or(""));
            if matches!(name, "text" | "mathrm" | "textrm" | "operatorname" | "textbf" | "mathbf" | "textit" | "mathit" | "mbox") {
                end += extract_braced_opt_ws(tail).map_or(0, |(_, len)| len);
            } else if matches!(name, "bar" | "overline" | "hat" | "widehat" | "tilde" | "widetilde") {
                end += extract_math_arg(tail, depth + 1)?.1;
            } else if name == "sqrt" {
                end += extract_sqrt_args(tail, depth + 1)?.2;
            } else if matches!(name, "frac" | "dfrac" | "tfrac" | "binom") {
                end += extract_two_math_args(tail, depth + 1)?.2;
            }
        }
        return Some((trimmed.get(..end)?, ws + end));
    }
    Some((trimmed.get(..ch.len_utf8())?, ws + ch.len_utf8()))
}

fn extract_two_math_args(s: &str, depth: usize) -> Option<(&str, &str, usize)> {
    if depth > MAX_MATH_DEPTH {
        return None;
    }
    let (a, ca) = extract_math_arg(s, depth + 1)?;
    let (b, cb) = extract_math_arg(s.get(ca..)?, depth + 1)?;
    Some((a, b, ca + cb))
}

fn extract_sqrt_args(s: &str, depth: usize) -> Option<(Option<&str>, &str, usize)> {
    if depth > MAX_MATH_DEPTH {
        return None;
    }
    let trimmed = s.trim_start_matches([' ', '\t']);
    let ws = s.len() - trimmed.len();
    if let Some(after) = trimmed.strip_prefix('[') {
        let mut bdepth = 0usize;
        for (idx, ch) in after.char_indices() {
            if idx >= MAX_SQRT_DEGREE_BYTES || ch == '\n' {
                break;
            }
            match ch {
                '{' => {
                    bdepth = bdepth.saturating_add(1);
                    if bdepth > MAX_MATH_DEPTH {
                        return None;
                    }
                }
                '}' => bdepth = bdepth.saturating_sub(1),
                ']' if bdepth == 0 => {
                    let (rad, rlen) = extract_math_arg(after.get(idx + 1..)?, depth + 1)?;
                    return Some((Some(after.get(..idx)?), rad, ws + idx + 2 + rlen));
                }
                _ => {}
            }
        }
    }
    let (rad, rlen) = extract_math_arg(s, depth + 1)?;
    Some((None, rad, rlen))
}

const ROMAN_MATH_FUNCTIONS: &str =
    "ln log exp sin cos tan sinh cosh tanh arcsin arccos arctan sec csc cot erf erfc min max lim sup inf arg det dim gcd lg";

const TEX_SYMBOL_PAIRS: &str = "\
alpha=α beta=β gamma=γ delta=δ epsilon=ε varepsilon=ε zeta=ζ eta=η theta=θ vartheta=ϑ iota=ι kappa=κ varkappa=ϰ lambda=λ mu=μ micro=μ nu=ν xi=ξ omicron=ο pi=π varpi=ϖ rho=ρ varrho=ϱ varsigma=ς sigma=σ tau=τ upsilon=υ phi=φ varphi=ϕ chi=χ psi=ψ omega=ω \
Alpha=Α Beta=Β Gamma=Γ Delta=Δ Epsilon=Ε Zeta=Ζ Eta=Η Theta=Θ Iota=Ι Kappa=Κ Lambda=Λ Mu=Μ Nu=Ν Xi=Ξ Omicron=Ο Pi=Π Rho=Ρ Sigma=Σ Tau=Τ Upsilon=Υ Phi=Φ Chi=Χ Psi=Ψ Omega=Ω \
ge=≥ geq=≥ le=≤ leq=≤ pm=± mp=∓ times=× div=÷ cdot=· bullet=• in=∈ notin=∉ ne=≠ neq=≠ infty=∞ propto=∝ approx=≈ equiv=≡ sim=∼ ll=≪ gg=≫ \
partial=∂ nabla=∇ sum=∑ prod=∏ int=∫ to=→ rightarrow=→ leftarrow=← leftrightarrow=↔ subset=⊂ subseteq=⊆ cup=∪ cap=∩ emptyset=∅ forall=∀ exists=∃ \
dots=… ldots=… cdots=… prime=′ dprime=″ degree=° deg=° textdegree=° langle=⟨ rangle=⟩ lceil=⌈ rceil=⌉ lfloor=⌊ rfloor=⌋ textbackslash=\\ textless=< textgreater=>";

fn lookup_tex_symbol(name: &str) -> Option<&'static str> {
    if name == "space" {
        return Some(" ");
    }
    TEX_SYMBOL_PAIRS.split_ascii_whitespace().find_map(|pair| pair.split_once('=').filter(|&(k, _)| k == name).map(|(_, v)| v))
}

pub fn push_html_escaped(raw: &str, out: &mut String) {
    raw.chars().for_each(|ch| push_char_escaped(ch, out));
}

#[must_use]
pub fn escape_html(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    push_html_escaped(raw, &mut out);
    out
}

fn push_char_escaped(ch: char, out: &mut String) {
    match ch {
        '&' => out.push_str("&amp;"),
        '<' => out.push_str("&lt;"),
        '>' => out.push_str("&gt;"),
        '"' => out.push_str("&quot;"),
        '\'' => out.push_str("&#39;"),
        other => out.push(other),
    }
}

#[cfg(test)]
mod tests {
    use super::{IdentityResolver, render_markdown, render_math_html};

    #[test]
    fn headings_images_urls_sentinels_ids_tables_and_math_spacing() {
        let r = render_markdown(
            "### Hougaard Skewness ($g_1$)\n\n## Chi-Square ($\\chi^2$)\n\n![Ratio $\\frac{a}{b}$](images/r.png)\n- ![I](images/i.png)\n",
            &IdentityResolver,
        );
        assert_eq!(
            (r.headings[0].id.as_str(), r.headings[0].text.as_str(), r.headings[1].text.as_str()),
            ("hougaard-skewness-g1", "Hougaard Skewness (g1)", "Chi-Square (χ2)")
        );
        assert!(r.html.contains("<figcaption>Ratio a/b</figcaption>") && r.html.contains("<li><img src=\"images/i.png\""));
        for md in ["[x](java&#x09;script:alert(1))", "[x](&#x01;javascript:alert(1))", "[x](data&#x09;:text/html,hi)"] {
            assert!(render_markdown(md, &IdentityResolver).html.contains("href=\"#\""), "{md}");
        }
        let sent =
            render_markdown("    echo $a$\n\n> ```sh\n> $$\n> x\n> $$\n> ```\n\nRaw \u{E000}CBM:0\u{E001} and $y_1$.\n", &IdentityResolver).html;
        assert!(sent.contains("echo $a$") && sent.contains("\u{E000}CBM:0\u{E001}") && sent.contains("<span class=\"math\">y<sub>1</sub></span>"));
        assert_eq!(
            render_markdown("# Foo\n\n## Foo\n\n## Foo 1\n\n## Main\n", &IdentityResolver).headings.into_iter().map(|h| h.id).collect::<Vec<_>>(),
            vec!["foo", "foo-1", "foo-1-1", "main-1"]
        );

        let h1_ren = render_markdown("Intro.\n\n# CLI (`curvebook-cli`) {#custom}\n\n## Usage\n", &IdentityResolver);
        let (pre, h1, post) = h1_ren.split_leading_h1();
        assert_eq!((pre, h1), ("<p>Intro.</p>\n", Some("<h1 id=\"custom\" tabindex=\"-1\">CLI (<code>curvebook-cli</code>)</h1>\n")));
        assert!(post.starts_with("<h2 id=\"usage\""));

        let tbl = render_markdown("| Op |\n|---|\n| `\\|\\|`, `OR` |\n", &IdentityResolver).html;
        assert!(tbl.contains("<td><code>||</code>, <code>OR</code></td>") && !tbl.contains('`'));
        let eq = render_math_html(
            "Y = \\text{Bottom} + \\frac{\\text{Top} - \\text{Bottom}}{1 + 10^{(\\log\\text{EC}_{50} - X) \\cdot \\text{HillSlope}}}",
        );
        assert!(
            eq.contains("<span class=\"math-text\">Bottom</span> + <span class=\"math-frac\">")
                && eq.contains("<sub>50</sub> - X) · <span class=\"math-text\">HillSlope</span>")
        );
        let note =
            render_markdown("> [!NOTE]\n> Body $\\frac{a}{b}$ and $100 and $HOME.\n\n$$\n\\chi^2 = \\frac{(O-E)^2}{E}\n$$\n", &IdentityResolver).html;
        assert!(note.contains("callout-note") && note.contains("$100") && note.contains("$HOME") && !note.contains("<em>"));

        let start = std::time::Instant::now();
        let _ = render_math_html(&"\\sqrt[".repeat(50_000));
        let _ = render_markdown(&"`".repeat(50_000), &IdentityResolver);
        let _ = render_markdown(&"$$a\n".repeat(50_000), &IdentityResolver);
        assert!(start.elapsed().as_secs() < 5 && !render_math_html(&"\\frac".repeat(80)).is_empty());
    }
}
