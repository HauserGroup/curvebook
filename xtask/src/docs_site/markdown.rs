//! Markdown-to-HTML compiler backed by `pulldown-cmark` with script-free sanitization,
//! GitHub-style heading anchors + `{#custom-id}`, GFM callouts, `<figure>` promotion,
//! and strict-CSP static TeX-math rendering (`R12.4` / `R12.5`).

use std::collections::BTreeMap;

use pulldown_cmark::{BlockQuoteKind, CodeBlockKind, Event, HeadingLevel, LinkType, Options, Parser, Tag, TagEnd};

/// One heading discovered while rendering a document page (powers the right-hand TOC).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TocHeading {
    pub level: u8,
    pub id: String,
    pub text: String,
}

/// Complete rendered HTML fragment + metadata extracted from one Markdown document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedMarkdown {
    pub title: Option<String>,
    pub headings: Vec<TocHeading>,
    pub html: String,
}

/// Rewrites relative `.md` links and asset paths into site URLs during rendering.
pub trait LinkResolver {
    fn resolve_href(&self, raw_href: &str) -> String;
    fn resolve_image(&self, raw_src: &str) -> String;
}

/// Default pass-through link resolver used in unit tests and standalone rendering.
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
}

const MATH_OPEN_SENTINEL: &str = "\u{E000}CBM:";
const MATH_CLOSE_SENTINEL: char = '\u{E001}';

/// Render a Markdown document into strict-CSP static HTML (zero `<script>`, zero inline `on*` handlers).
pub fn render_markdown(input: &str, resolver: &dyn LinkResolver) -> RenderedMarkdown {
    let normalized = input.replace("\r\n", "\n");
    let (protected, math_slots) = protect_math_spans(&normalized);
    let parser = Parser::new_ext(&protected, markdown_options());
    let mut renderer = HtmlRenderer {
        resolver,
        math_slots: &math_slots,
        slug_counts: BTreeMap::new(),
        headings: Vec::new(),
        title: None,
        out: String::with_capacity(normalized.len().saturating_mul(2)),
        blockquote_ends: Vec::new(),
        in_table_head: false,
        table_tbody_open: false,
        in_code_block: false,
    };
    renderer.render_stream(parser);
    RenderedMarkdown { title: renderer.title, headings: renderer.headings, html: renderer.out }
}

fn markdown_options() -> Options {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_GFM);
    opts.insert(Options::ENABLE_HEADING_ATTRIBUTES);
    opts.insert(Options::ENABLE_TASKLISTS);
    opts
}

struct HtmlRenderer<'a> {
    resolver: &'a dyn LinkResolver,
    math_slots: &'a [MathSlot],
    slug_counts: BTreeMap<String, usize>,
    headings: Vec<TocHeading>,
    title: Option<String>,
    out: String,
    blockquote_ends: Vec<&'static str>,
    in_table_head: bool,
    table_tbody_open: bool,
    in_code_block: bool,
}

impl<'a> HtmlRenderer<'a> {
    fn unique_id(&mut self, base: &str) -> String {
        let norm = if base.is_empty() { String::from("section") } else { base.to_owned() };
        let count = self.slug_counts.entry(norm.clone()).or_insert(0);
        let id = if *count == 0 { norm } else { format!("{norm}-{}", *count) };
        *count += 1;
        id
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
                    let explicit_id = id.map(|c| c.into_string());
                    self.flush_heading(level, explicit_id.as_deref(), inner);
                }
                Event::Start(tag) => self.start_tag(tag),
                Event::End(tag) => self.end_tag(tag),
                other => self.emit_leaf_event(other, false),
            }
        }
    }

    fn flush_paragraph(&mut self, events: Vec<Event<'_>>) {
        if let Some(slot_id) = standalone_display_math_slot(&events)
            && let Some(slot) = self.math_slots.get(slot_id)
            && slot.kind == MathKind::DisplayBlock
        {
            self.out.push_str("<div class=\"math-block\"><span class=\"math\">");
            render_tex_math_inner(slot.tex.trim(), 0, &mut self.out);
            self.out.push_str("</span></div>\n");
            return;
        }
        if let Some((dest_url, alt)) = standalone_figure_image(&events, self.math_slots) {
            let resolved = sanitize_resolved_url(&self.resolver.resolve_image(&dest_url));
            self.out.push_str("<figure class=\"doc-figure\"><img src=\"");
            push_html_escaped(&resolved, &mut self.out);
            self.out.push_str("\" alt=\"");
            push_html_escaped(&alt, &mut self.out);
            self.out.push_str("\" loading=\"lazy\">");
            let cap = alt.trim();
            if !cap.is_empty() {
                self.out.push_str("<figcaption>");
                push_html_escaped(cap, &mut self.out);
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
        let plain = events_to_plain_text(&events, self.math_slots);
        let lvl = heading_level_u8(level);
        if lvl == 1 && self.title.is_none() {
            self.title = Some(plain.clone());
        }
        let base_slug = match explicit_id {
            Some(id) if !id.is_empty() => id.to_owned(),
            _ => github_slug(&plain),
        };
        let id = self.unique_id(&base_slug);
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
                Event::Start(Tag::Image { dest_url, .. }) => {
                    let alt_events = collect_until_end(&mut iter, |t| matches!(t, TagEnd::Image));
                    let alt = events_to_plain_text(&alt_events, self.math_slots);
                    let resolved = sanitize_resolved_url(&self.resolver.resolve_image(&dest_url));
                    self.out.push_str("<img src=\"");
                    push_html_escaped(&resolved, &mut self.out);
                    self.out.push_str("\" alt=\"");
                    push_html_escaped(&alt, &mut self.out);
                    self.out.push_str("\" loading=\"lazy\">");
                }
                Event::Start(tag) => self.start_tag(tag),
                Event::End(tag) => self.end_tag(tag),
                other => self.emit_leaf_event(other, true),
            }
        }
    }

    fn start_tag(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::BlockQuote(kind) => self.start_blockquote(kind),
            Tag::CodeBlock(kind) => self.start_code_block(kind),
            Tag::List(None) => self.out.push_str("<ul>\n"),
            Tag::List(Some(1)) => self.out.push_str("<ol>\n"),
            Tag::List(Some(start)) => self.out.push_str(&format!("<ol start=\"{start}\">\n")),
            Tag::Item => self.out.push_str("<li>"),
            Tag::Table(_) => {
                self.table_tbody_open = false;
                self.out.push_str("<div class=\"table-wrap\"><table>\n");
            }
            Tag::TableHead => {
                self.in_table_head = true;
                self.out.push_str("<thead><tr>");
            }
            Tag::TableRow => {
                if !self.table_tbody_open {
                    self.out.push_str("<tbody>\n");
                    self.table_tbody_open = true;
                }
                self.out.push_str("<tr>");
            }
            Tag::TableCell => self.out.push_str(if self.in_table_head { "<th>" } else { "<td>" }),
            Tag::Emphasis => self.out.push_str("<em>"),
            Tag::Strong => self.out.push_str("<strong>"),
            Tag::Strikethrough => self.out.push_str("<del>"),
            Tag::Link { link_type, dest_url, title, .. } => self.start_link(link_type, &dest_url, &title),
            Tag::Superscript => self.out.push_str("<sup>"),
            Tag::Subscript => self.out.push_str("<sub>"),
            Tag::HtmlBlock
            | Tag::Paragraph
            | Tag::Heading { .. }
            | Tag::Image { .. }
            | Tag::FootnoteDefinition(_)
            | Tag::MetadataBlock(_)
            | Tag::DefinitionList
            | Tag::DefinitionListTitle
            | Tag::DefinitionListDefinition => {}
        }
    }

    fn start_blockquote(&mut self, kind: Option<BlockQuoteKind>) {
        if let Some(alert) = kind {
            let (slug, title) = match alert {
                BlockQuoteKind::Note => ("note", "Note"),
                BlockQuoteKind::Tip => ("tip", "Tip"),
                BlockQuoteKind::Important => ("important", "Important"),
                BlockQuoteKind::Warning => ("warning", "Warning"),
                BlockQuoteKind::Caution => ("caution", "Caution"),
            };
            self.out.push_str("<div class=\"callout callout-");
            self.out.push_str(slug);
            self.out.push_str("\" role=\"note\">\n<p class=\"callout-title\">");
            self.out.push_str(title);
            self.out.push_str("</p>\n");
            self.blockquote_ends.push("</div>\n");
        } else {
            self.out.push_str("<blockquote>\n");
            self.blockquote_ends.push("</blockquote>\n");
        }
    }

    fn start_code_block(&mut self, kind: CodeBlockKind<'_>) {
        self.in_code_block = true;
        match kind {
            CodeBlockKind::Fenced(info) => {
                let lang = info.split_whitespace().next().unwrap_or("");
                if lang.is_empty() {
                    self.out.push_str("<pre><code>");
                } else {
                    self.out.push_str("<pre><code class=\"language-");
                    push_html_escaped(lang, &mut self.out);
                    self.out.push_str("\">");
                }
            }
            CodeBlockKind::Indented => self.out.push_str("<pre><code>"),
        }
    }

    fn start_link(&mut self, link_type: LinkType, dest_url: &str, title: &str) {
        let raw_href =
            if link_type == LinkType::Email && !dest_url.starts_with("mailto:") { format!("mailto:{dest_url}") } else { dest_url.to_owned() };
        let resolved = sanitize_resolved_url(&self.resolver.resolve_href(&raw_href));
        self.out.push_str("<a href=\"");
        push_html_escaped(&resolved, &mut self.out);
        self.out.push('"');
        if !title.is_empty() {
            self.out.push_str(" title=\"");
            push_html_escaped(title, &mut self.out);
            self.out.push('"');
        }
        self.out.push('>');
    }

    fn end_tag(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::BlockQuote(_) => {
                if let Some(close) = self.blockquote_ends.pop() {
                    self.out.push_str(close);
                }
            }
            TagEnd::CodeBlock => {
                self.in_code_block = false;
                self.out.push_str("</code></pre>\n");
            }
            TagEnd::List(true) => self.out.push_str("</ol>\n"),
            TagEnd::List(false) => self.out.push_str("</ul>\n"),
            TagEnd::Item => self.out.push_str("</li>\n"),
            TagEnd::TableHead => {
                self.out.push_str("</tr></thead>\n");
                self.in_table_head = false;
            }
            TagEnd::TableRow => self.out.push_str("</tr>\n"),
            TagEnd::TableCell => self.out.push_str(if self.in_table_head { "</th>" } else { "</td>" }),
            TagEnd::Table => {
                if self.table_tbody_open {
                    self.out.push_str("</tbody>\n");
                    self.table_tbody_open = false;
                }
                self.out.push_str("</table></div>\n");
            }
            TagEnd::Emphasis => self.out.push_str("</em>"),
            TagEnd::Strong => self.out.push_str("</strong>"),
            TagEnd::Strikethrough => self.out.push_str("</del>"),
            TagEnd::Link => self.out.push_str("</a>"),
            TagEnd::Superscript => self.out.push_str("</sup>"),
            TagEnd::Subscript => self.out.push_str("</sub>"),
            TagEnd::Paragraph
            | TagEnd::Heading(_)
            | TagEnd::Image
            | TagEnd::HtmlBlock
            | TagEnd::FootnoteDefinition
            | TagEnd::MetadataBlock(_)
            | TagEnd::DefinitionList
            | TagEnd::DefinitionListTitle
            | TagEnd::DefinitionListDefinition => {}
        }
    }

    fn emit_leaf_event(&mut self, event: Event<'_>, inline_context: bool) {
        match event {
            Event::Text(text) => {
                if self.in_code_block {
                    push_html_escaped(&text, &mut self.out);
                } else {
                    render_text_with_math(&text, self.math_slots, &mut self.out);
                }
            }
            Event::Code(code) => {
                self.out.push_str("<code>");
                push_html_escaped(&code, &mut self.out);
                self.out.push_str("</code>");
            }
            Event::InlineMath(tex) => {
                self.out.push_str("<span class=\"math\">");
                render_tex_math_inner(tex.trim(), 0, &mut self.out);
                self.out.push_str("</span>");
            }
            Event::DisplayMath(tex) => {
                self.out.push_str("<div class=\"math-block\"><span class=\"math\">");
                render_tex_math_inner(tex.trim(), 0, &mut self.out);
                self.out.push_str("</span></div>\n");
            }
            Event::SoftBreak => self.out.push(if inline_context { ' ' } else { '\n' }),
            Event::HardBreak => self.out.push_str("<br>\n"),
            Event::Rule => self.out.push_str("<hr>\n"),
            Event::Html(raw) | Event::InlineHtml(raw) => emit_sanitized_html_fragment(&raw, &mut self.out),
            Event::TaskListMarker(checked) => {
                if checked {
                    self.out.push_str("<input type=\"checkbox\" disabled checked> ");
                } else {
                    self.out.push_str("<input type=\"checkbox\" disabled> ");
                }
            }
            Event::FootnoteReference(name) => {
                self.out.push_str("<sup>[");
                push_html_escaped(&name, &mut self.out);
                self.out.push_str("]</sup>");
            }
            Event::Start(_) | Event::End(_) => {}
        }
    }
}

fn collect_until_end<'e>(iter: &mut impl Iterator<Item = Event<'e>>, is_end: impl Fn(&TagEnd) -> bool) -> Vec<Event<'e>> {
    let mut events = Vec::new();
    let mut depth = 1usize;
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

fn standalone_display_math_slot(events: &[Event<'_>]) -> Option<usize> {
    let [Event::Text(text)] = events else {
        return None;
    };
    let trimmed = text.trim();
    let rest = trimmed.strip_prefix(MATH_OPEN_SENTINEL)?;
    let (digits, after) = rest.split_once(MATH_CLOSE_SENTINEL)?;
    if !after.is_empty() {
        return None;
    }
    digits.parse::<usize>().ok()
}

fn standalone_figure_image(events: &[Event<'_>], slots: &[MathSlot]) -> Option<(String, String)> {
    let first = events.first()?;
    let Event::Start(Tag::Image { dest_url, .. }) = first else {
        return None;
    };
    let Some(Event::End(TagEnd::Image)) = events.last() else {
        return None;
    };
    let mut depth = 1usize;
    for ev in events.get(1..events.len().saturating_sub(1))? {
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
    let alt = events_to_plain_text(events.get(1..events.len().saturating_sub(1))?, slots);
    Some((dest_url.to_string(), alt))
}

fn events_to_plain_text(events: &[Event<'_>], slots: &[MathSlot]) -> String {
    let mut out = String::new();
    for ev in events {
        match ev {
            Event::Text(t) => append_plain_text_with_math(t, slots, &mut out),
            Event::Code(c) => out.push_str(c),
            Event::InlineMath(m) | Event::DisplayMath(m) => out.push_str(m.trim()),
            Event::SoftBreak | Event::HardBreak => out.push(' '),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn append_plain_text_with_math(text: &str, slots: &[MathSlot], out: &mut String) {
    let mut rest = text;
    while let Some((before, after_open)) = rest.split_once(MATH_OPEN_SENTINEL) {
        out.push_str(before);
        if let Some((digits, tail)) = after_open.split_once(MATH_CLOSE_SENTINEL)
            && let Ok(idx) = digits.parse::<usize>()
            && let Some(slot) = slots.get(idx)
        {
            out.push_str(slot.tex.trim());
            rest = tail;
            continue;
        }
        out.push_str(MATH_OPEN_SENTINEL);
        rest = after_open;
    }
    out.push_str(rest);
}

fn render_text_with_math(text: &str, slots: &[MathSlot], out: &mut String) {
    let mut rest = text;
    while let Some((before, after_open)) = rest.split_once(MATH_OPEN_SENTINEL) {
        push_html_escaped(before, out);
        if let Some((digits, tail)) = after_open.split_once(MATH_CLOSE_SENTINEL)
            && let Ok(idx) = digits.parse::<usize>()
            && let Some(slot) = slots.get(idx)
        {
            let (open, close) = match slot.kind {
                MathKind::DisplayBlock => ("<div class=\"math-block\"><span class=\"math\">", "</span></div>"),
                MathKind::InlineBlockSpan => ("<span class=\"math math-block\">", "</span>"),
                MathKind::Inline => ("<span class=\"math\">", "</span>"),
            };
            out.push_str(open);
            render_tex_math_inner(slot.tex.trim(), 0, out);
            out.push_str(close);
            rest = tail;
            continue;
        }
        push_html_escaped(MATH_OPEN_SENTINEL, out);
        rest = after_open;
    }
    push_html_escaped(rest, out);
}

fn emit_sanitized_html_fragment(raw: &str, out: &mut String) {
    let trimmed = raw.trim();
    if trimmed.is_empty() || (trimmed.starts_with("<!--") && trimmed.ends_with("-->")) {
        return;
    }
    let lower = trimmed.to_ascii_lowercase();
    if matches!(lower.as_str(), "<br>" | "<br/>" | "<br />") {
        out.push_str("<br>");
    } else {
        push_html_escaped(raw, out);
    }
}

fn sanitize_resolved_url(url: &str) -> String {
    let trimmed = url.trim();
    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("javascript:") || lower.starts_with("vbscript:") || lower.starts_with("data:") {
        String::from("#")
    } else {
        trimmed.to_owned()
    }
}

const fn heading_level_u8(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

/// Compute a GitHub-style slug from plain heading text (`A–Z` -> `a–z`, space -> `-`).
#[must_use]
pub fn github_slug(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
            out.push(ch.to_ascii_lowercase());
        } else if ch == ' ' || ch == '\t' {
            out.push('-');
        }
    }
    out.trim_matches('-').to_owned()
}

fn parse_fence_open(trimmed: &str) -> Option<(char, usize, String)> {
    let first = trimmed.chars().next()?;
    if first != '`' && first != '~' {
        return None;
    }
    let count = trimmed.bytes().take_while(|&b| b == first as u8).count();
    if count < 3 {
        return None;
    }
    let info = trimmed.get(count..)?.trim().to_owned();
    if first == '`' && info.contains('`') {
        return None;
    }
    Some((first, count, info))
}

fn is_fence_close(trimmed: &str, ch: char, min_len: usize) -> bool {
    let count = trimmed.bytes().take_while(|&b| b == ch as u8).count();
    count >= min_len && trimmed.get(count..).is_some_and(|rest| rest.trim().is_empty())
}

fn protect_math_spans(input: &str) -> (String, Vec<MathSlot>) {
    let lines: Vec<&str> = input.lines().collect();
    let mut out = String::with_capacity(input.len() + 64);
    let mut slots = Vec::new();
    let mut fence: Option<(char, usize)> = None;
    let mut idx = 0usize;

    while let Some(&line) = lines.get(idx) {
        let trimmed = line.trim();
        if let Some((ch, min_len)) = fence {
            out.push_str(line);
            out.push('\n');
            if is_fence_close(trimmed, ch, min_len) {
                fence = None;
            }
            idx += 1;
            continue;
        }
        if let Some((ch, len, _)) = parse_fence_open(trimmed) {
            fence = Some((ch, len));
            out.push_str(line);
            out.push('\n');
            idx += 1;
            continue;
        }
        if is_display_math_start(&lines, idx)
            && let Some((tex, next_idx)) = extract_display_math_block(&lines, idx)
        {
            let id = slots.len();
            slots.push(MathSlot { kind: MathKind::DisplayBlock, tex });
            out.push('\n');
            out.push_str(&format!("{MATH_OPEN_SENTINEL}{id}{MATH_CLOSE_SENTINEL}"));
            out.push_str("\n\n");
            idx = next_idx;
            continue;
        }
        protect_line_math(line, &mut slots, &mut out);
        out.push('\n');
        idx += 1;
    }
    (out, slots)
}

fn extract_display_math_block(lines: &[&str], idx: usize) -> Option<(String, usize)> {
    let trimmed = lines.get(idx)?.trim();
    let after_open = trimmed.strip_prefix("$$")?;
    if let Some(inner) = after_open.strip_suffix("$$") {
        let clean = inner.trim();
        if inner.contains("$$") || clean.is_empty() {
            return None;
        }
        return Some((clean.to_owned(), idx + 1));
    }
    if after_open.contains("$$") {
        return None;
    }
    let close_idx = find_display_math_close(lines, idx + 1)?;
    let mut parts: Vec<&str> = Vec::new();
    let head = after_open.trim();
    if !head.is_empty() {
        parts.push(head);
    }
    for line in lines.get(idx + 1..close_idx)? {
        let t = line.trim();
        if !t.is_empty() {
            parts.push(t);
        }
    }
    let close_trimmed = lines.get(close_idx)?.trim();
    if let Some(tail) = close_trimmed.strip_suffix("$$") {
        let tail_t = tail.trim();
        if !tail_t.is_empty() {
            parts.push(tail_t);
        }
    }
    let joined = parts.join(" ");
    if joined.trim().is_empty() {
        return None;
    }
    Some((joined.trim().to_owned(), close_idx + 1))
}

fn protect_line_math(line: &str, slots: &mut Vec<MathSlot>, out: &mut String) {
    let mut i = 0usize;
    while let Some(rest) = line.get(i..) {
        if rest.is_empty() {
            break;
        }
        let Some(ch) = rest.chars().next() else {
            break;
        };
        if ch == '\\' {
            out.push('\\');
            i += 1;
            if let Some(next_ch) = line.get(i..).and_then(|s| s.chars().next()) {
                out.push(next_ch);
                i += next_ch.len_utf8();
            }
            continue;
        }
        if ch == '`'
            && let Some((_, consumed)) = parse_code_span(rest)
            && let Some(raw_span) = rest.get(..consumed)
        {
            out.push_str(raw_span);
            i += consumed;
            continue;
        }
        if rest.starts_with("$$")
            && let Some((tex, consumed)) = parse_block_math_inline(rest)
        {
            let id = slots.len();
            slots.push(MathSlot { kind: MathKind::InlineBlockSpan, tex: tex.to_owned() });
            out.push_str(&format!("{MATH_OPEN_SENTINEL}{id}{MATH_CLOSE_SENTINEL}"));
            i += consumed;
            continue;
        }
        if ch == '$'
            && !line.get(..i).and_then(|p| p.chars().next_back()).is_some_and(|c| c.is_ascii_alphanumeric())
            && let Some((tex, consumed)) = parse_inline_math(rest)
        {
            let id = slots.len();
            slots.push(MathSlot { kind: MathKind::Inline, tex: tex.to_owned() });
            out.push_str(&format!("{MATH_OPEN_SENTINEL}{id}{MATH_CLOSE_SENTINEL}"));
            i += consumed;
            continue;
        }
        out.push(ch);
        i += ch.len_utf8();
    }
}

fn parse_code_span(slice: &str) -> Option<(&str, usize)> {
    let ticks = slice.bytes().take_while(|&b| b == b'`').count();
    if ticks == 0 {
        return None;
    }
    let after_open = slice.get(ticks..)?;
    let mut pos = 0usize;
    while let Some(cur) = after_open.get(pos..) {
        if cur.is_empty() {
            break;
        }
        if cur.starts_with('`') {
            let run = cur.bytes().take_while(|&b| b == b'`').count();
            if run == ticks {
                let raw = after_open.get(..pos)?;
                let trimmed = raw.strip_prefix(' ').and_then(|s| s.strip_suffix(' ')).unwrap_or(raw);
                return Some((trimmed, ticks + pos + ticks));
            }
            pos += run;
            continue;
        }
        let ch = cur.chars().next()?;
        pos += ch.len_utf8();
    }
    None
}

/// Render a raw TeX-subset formula into a strict-CSP static `<span class="math">...</span>`.
#[must_use]
pub fn render_math_html(tex: &str) -> String {
    let trimmed = tex.trim();
    let mut out = String::with_capacity(trimmed.len().saturating_mul(2) + 32);
    out.push_str("<span class=\"math\">");
    render_tex_math_inner(trimmed, 0, &mut out);
    out.push_str("</span>");
    out
}

const MAX_MATH_DEPTH: usize = 32;

fn is_display_math_start(lines: &[&str], idx: usize) -> bool {
    extract_display_math_block(lines, idx).is_some()
}

fn find_display_math_close(lines: &[&str], start: usize) -> Option<usize> {
    for (offset, line) in lines.get(start..)?.iter().enumerate() {
        let trimmed = line.trim();
        if parse_fence_open(trimmed).is_some() {
            return None;
        }
        if trimmed == "$$" || (trimmed.ends_with("$$") && !ends_with_escaped_dollars(trimmed)) {
            return Some(start + offset);
        }
    }
    None
}

fn ends_with_escaped_dollars(trimmed: &str) -> bool {
    let Some(before) = trimmed.strip_suffix("$$") else {
        return false;
    };
    let backslashes = before.bytes().rev().take_while(|&b| b == b'\\').count();
    backslashes % 2 == 1
}

fn parse_delimited_math(rest: &str, block: bool) -> Option<(&str, usize)> {
    let delim = if block { "$$" } else { "$" };
    let after_open = rest.strip_prefix(delim)?;
    if !block && (after_open.is_empty() || after_open.starts_with('$') || after_open.chars().next().is_some_and(char::is_whitespace)) {
        return None;
    }
    let mut i = 0usize;
    let mut brace_depth = 0usize;
    while let Some(cur) = after_open.get(i..) {
        let ch = cur.chars().next()?;
        if ch == '\\' {
            i += 1;
            if let Some(nc) = after_open.get(i..).and_then(|s| s.chars().next()) {
                i += nc.len_utf8();
            }
            continue;
        }
        if ch == '`' && parse_code_span(cur).is_some() {
            return None;
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
                let inner = after_open.get(..i)?;
                if block {
                    if inner.trim().is_empty() {
                        return None;
                    }
                    return Some((inner, i + 4));
                }
                let after_close = after_open.get(i + 1..)?;
                if inner.is_empty()
                    || inner.chars().last().is_some_and(char::is_whitespace)
                    || after_close.starts_with('$')
                    || after_close.chars().next().is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
                {
                    return None;
                }
                return Some((inner, i + 2));
            }
            _ => {}
        }
        i += ch.len_utf8();
    }
    None
}

fn parse_block_math_inline(rest: &str) -> Option<(&str, usize)> {
    parse_delimited_math(rest, true)
}

fn parse_inline_math(rest: &str) -> Option<(&str, usize)> {
    parse_delimited_math(rest, false)
}

fn render_tex_math_inner(tex: &str, depth: usize, out: &mut String) {
    if depth > MAX_MATH_DEPTH {
        push_html_escaped(tex, out);
        return;
    }
    let mut i = 0usize;
    while let Some(rest) = tex.get(i..) {
        let Some(ch) = rest.chars().next() else {
            break;
        };
        if ch.is_whitespace() {
            while let Some(nc) = tex.get(i..).and_then(|s| s.chars().next())
                && nc.is_whitespace()
            {
                i += nc.len_utf8();
            }
            if !out.ends_with(' ') && !out.is_empty() && !out.ends_with('>') {
                out.push(' ');
            }
            continue;
        }
        if ch == '{'
            && let Some((inner, consumed)) = extract_braced(rest)
        {
            render_tex_math_inner(inner, depth + 1, out);
            i += consumed;
            continue;
        }
        if ch == '_' || ch == '^' {
            let tag = if ch == '_' { "sub" } else { "sup" };
            let after = rest.get(1..).unwrap_or("");
            if let Some((arg, consumed)) = extract_math_arg(after, depth) {
                out.push('<');
                out.push_str(tag);
                out.push('>');
                render_tex_math_inner(arg, depth + 1, out);
                out.push_str("</");
                out.push_str(tag);
                out.push('>');
                i += 1 + consumed;
                continue;
            }
        }
        if ch == '\\' {
            i += render_tex_command(rest, depth, out);
            continue;
        }
        push_char_escaped(ch, out);
        i += ch.len_utf8();
    }
}

fn render_tex_command(rest: &str, depth: usize, out: &mut String) -> usize {
    let Some(after) = rest.get(1..) else {
        return 1;
    };
    let Some(first) = after.chars().next() else {
        return 1;
    };
    if !first.is_ascii_alphabetic() {
        match first {
            ',' => out.push('\u{2009}'),
            '!' => {}
            ';' | ':' | ' ' | '\\' => out.push(' '),
            '|' => out.push('\u{2016}'),
            _ => push_char_escaped(first, out),
        }
        return 1 + first.len_utf8();
    }
    let name_len = after.bytes().take_while(u8::is_ascii_alphabetic).count();
    let name = after.get(..name_len).unwrap_or("");
    let tail = after.get(name_len..).unwrap_or("");

    if matches!(name, "frac" | "dfrac" | "tfrac")
        && let Some((num, den, consumed)) = extract_two_math_args(tail, depth)
    {
        out.push_str("<span class=\"math-frac\"><span class=\"math-num\">");
        render_tex_math_inner(num.trim(), depth + 1, out);
        out.push_str("</span><span class=\"math-den\">");
        render_tex_math_inner(den.trim(), depth + 1, out);
        out.push_str("</span></span>");
        return 1 + name_len + consumed;
    }
    if name == "binom"
        && let Some((num, den, consumed)) = extract_two_math_args(tail, depth)
    {
        out.push('(');
        render_tex_math_inner(num.trim(), depth + 1, out);
        out.push_str(" choose ");
        render_tex_math_inner(den.trim(), depth + 1, out);
        out.push(')');
        return 1 + name_len + consumed;
    }
    if name == "sqrt"
        && let Some((deg, rad, consumed)) = extract_sqrt_args(tail, depth)
    {
        if let Some(d) = deg {
            out.push_str("<sup>");
            render_tex_math_inner(d.trim(), depth + 1, out);
            out.push_str("</sup>");
        }
        out.push_str("\u{221A}<span class=\"math-rad\">");
        render_tex_math_inner(rad.trim(), depth + 1, out);
        out.push_str("</span>");
        return 1 + name_len + consumed;
    }
    if matches!(name, "text" | "mathrm" | "textrm" | "operatorname" | "textbf" | "mathbf" | "mbox")
        && let Some((inner, consumed)) = extract_braced_opt_ws(tail)
    {
        out.push_str("<span class=\"math-text\">");
        render_tex_text_mode(inner, out);
        out.push_str("</span>");
        return 1 + name_len + consumed;
    }
    if matches!(name, "textit" | "mathit")
        && let Some((inner, consumed)) = extract_braced_opt_ws(tail)
    {
        out.push_str("<em>");
        render_tex_text_mode(inner, out);
        out.push_str("</em>");
        return 1 + name_len + consumed;
    }
    if matches!(name, "bar" | "overline" | "hat" | "widehat" | "tilde" | "widetilde")
        && let Some((arg, consumed)) = extract_math_arg(tail, depth)
    {
        let accent = match name {
            "bar" | "overline" => '\u{0304}',
            "hat" | "widehat" => '\u{0302}',
            _ => '\u{0303}',
        };
        let mut inner_html = String::new();
        render_tex_math_inner(arg.trim(), depth + 1, &mut inner_html);
        push_accented_html(&inner_html, accent, out);
        return 1 + name_len + consumed;
    }
    if matches!(name, "left" | "right") {
        let extra = if tail.starts_with('.') { 1 } else { 0 };
        return 1 + name_len + extra;
    }
    if name == "TeX" {
        out.push_str("<span class=\"math-text\">TeX</span>");
        return 1 + name_len;
    }
    if name == "LaTeX" {
        out.push_str("<span class=\"math-text\">LaTeX</span>");
        return 1 + name_len;
    }
    if name == "quad" {
        out.push('\u{2003}');
        return 1 + name_len;
    }
    if name == "qquad" {
        out.push_str("\u{2003}\u{2003}");
        return 1 + name_len;
    }
    if is_roman_math_function(name) {
        out.push_str("<span class=\"math-text\">");
        out.push_str(name);
        out.push_str("</span>");
        return 1 + name_len;
    }
    if let Some(sym) = lookup_tex_symbol(name) {
        let extra = if matches!(name, "textbackslash" | "textless" | "textgreater" | "textdegree") && tail.starts_with("{}") { 2 } else { 0 };
        push_html_escaped(sym, out);
        return 1 + name_len + extra;
    }

    push_html_escaped(name, out);
    1 + name_len
}

fn render_tex_text_mode(inner: &str, out: &mut String) {
    let mut i = 0usize;
    while let Some(rest) = inner.get(i..) {
        let Some(ch) = rest.chars().next() else {
            break;
        };
        if ch == '\\'
            && let Some(next_ch) = rest.get(1..).and_then(|s| s.chars().next())
        {
            if next_ch.is_ascii_punctuation() || next_ch == ' ' {
                push_char_escaped(next_ch, out);
                i += 1 + next_ch.len_utf8();
                continue;
            }
            let after = rest.get(1..).unwrap_or("");
            let nlen = after.bytes().take_while(u8::is_ascii_alphabetic).count();
            if nlen > 0
                && let Some(sym) = lookup_tex_symbol(after.get(..nlen).unwrap_or(""))
            {
                push_html_escaped(sym, out);
                i += 1 + nlen;
                continue;
            }
        }
        push_char_escaped(ch, out);
        i += ch.len_utf8();
    }
}

fn push_accented_html(rendered: &str, accent: char, out: &mut String) {
    let mut inserted = false;
    let mut in_tag = false;
    let mut in_entity = false;
    for ch in rendered.chars() {
        out.push(ch);
        if inserted {
            continue;
        }
        if ch == '<' {
            in_tag = true;
        } else if in_tag {
            if ch == '>' {
                in_tag = false;
            }
        } else if ch == '&' {
            in_entity = true;
        } else if in_entity {
            if ch == ';' {
                in_entity = false;
                out.push(accent);
                inserted = true;
            }
        } else if !ch.is_whitespace() {
            out.push(accent);
            inserted = true;
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
    let ws = s.len() - trimmed.len();
    let (inner, consumed) = extract_braced(trimmed)?;
    Some((inner, ws + consumed))
}

fn extract_math_arg(s: &str, depth: usize) -> Option<(&str, usize)> {
    if depth > MAX_MATH_DEPTH {
        return None;
    }
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
        let mut end = if nlen > 0 { 1 + nlen } else { 1 + after.chars().next().map_or(0, char::len_utf8) };
        if nlen > 0 {
            let name = after.get(..nlen).unwrap_or("");
            let tail = after.get(nlen..).unwrap_or("");
            if matches!(name, "text" | "mathrm" | "textrm" | "operatorname" | "textbf" | "mathbf" | "textit" | "mathit" | "mbox") {
                if let Some((_, arg_len)) = extract_braced_opt_ws(tail) {
                    end += arg_len;
                }
            } else if matches!(name, "bar" | "overline" | "hat" | "widehat" | "tilde" | "widetilde") {
                let (_, arg_len) = extract_math_arg(tail, depth + 1)?;
                end += arg_len;
            } else if name == "sqrt" {
                let (_, _, arg_len) = extract_sqrt_args(tail, depth + 1)?;
                end += arg_len;
            } else if matches!(name, "frac" | "dfrac" | "tfrac" | "binom") {
                let (_, _, arg_len) = extract_two_math_args(tail, depth + 1)?;
                end += arg_len;
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
    if let Some(after_bracket) = trimmed.strip_prefix('[') {
        let mut brace_depth = 0usize;
        for (idx, ch) in after_bracket.char_indices() {
            match ch {
                '{' => {
                    brace_depth = brace_depth.saturating_add(1);
                    if brace_depth > MAX_MATH_DEPTH {
                        return None;
                    }
                }
                '}' => brace_depth = brace_depth.saturating_sub(1),
                ']' if brace_depth == 0 => {
                    let deg = after_bracket.get(..idx)?;
                    let rest = after_bracket.get(idx + 1..)?;
                    let (rad, rad_len) = extract_math_arg(rest, depth + 1)?;
                    return Some((Some(deg), rad, ws + 1 + idx + 1 + rad_len));
                }
                _ => {}
            }
        }
    }
    let (rad, rad_len) = extract_math_arg(s, depth + 1)?;
    Some((None, rad, rad_len))
}

const ROMAN_MATH_FUNCTIONS: &[&str] = &[
    "ln", "log", "exp", "sin", "cos", "tan", "sinh", "cosh", "tanh", "arcsin", "arccos", "arctan", "sec", "csc", "cot", "erf", "erfc", "min", "max",
    "lim", "sup", "inf", "arg", "det", "dim", "gcd", "lg",
];

fn is_roman_math_function(name: &str) -> bool {
    ROMAN_MATH_FUNCTIONS.contains(&name)
}

const TEX_SYMBOLS: &[(&str, &str)] = &[
    ("alpha", "α"),
    ("beta", "β"),
    ("gamma", "γ"),
    ("delta", "δ"),
    ("epsilon", "ε"),
    ("varepsilon", "ε"),
    ("zeta", "ζ"),
    ("eta", "η"),
    ("theta", "θ"),
    ("vartheta", "ϑ"),
    ("iota", "ι"),
    ("kappa", "κ"),
    ("varkappa", "ϰ"),
    ("lambda", "λ"),
    ("mu", "μ"),
    ("micro", "μ"),
    ("nu", "ν"),
    ("xi", "ξ"),
    ("omicron", "ο"),
    ("pi", "π"),
    ("varpi", "ϖ"),
    ("rho", "ρ"),
    ("varrho", "ϱ"),
    ("varsigma", "ς"),
    ("sigma", "σ"),
    ("tau", "τ"),
    ("upsilon", "υ"),
    ("phi", "φ"),
    ("varphi", "ϕ"),
    ("chi", "χ"),
    ("psi", "ψ"),
    ("omega", "ω"),
    ("Alpha", "Α"),
    ("Beta", "Β"),
    ("Gamma", "Γ"),
    ("Delta", "Δ"),
    ("Epsilon", "Ε"),
    ("Zeta", "Ζ"),
    ("Eta", "Η"),
    ("Theta", "Θ"),
    ("Iota", "Ι"),
    ("Kappa", "Κ"),
    ("Lambda", "Λ"),
    ("Mu", "Μ"),
    ("Nu", "Ν"),
    ("Xi", "Ξ"),
    ("Omicron", "Ο"),
    ("Pi", "Π"),
    ("Rho", "Ρ"),
    ("Sigma", "Σ"),
    ("Tau", "Τ"),
    ("Upsilon", "Υ"),
    ("Phi", "Φ"),
    ("Chi", "Χ"),
    ("Psi", "Ψ"),
    ("Omega", "Ω"),
    ("ge", "≥"),
    ("geq", "≥"),
    ("le", "≤"),
    ("leq", "≤"),
    ("pm", "±"),
    ("mp", "∓"),
    ("times", "×"),
    ("div", "÷"),
    ("cdot", "·"),
    ("bullet", "•"),
    ("in", "∈"),
    ("notin", "∉"),
    ("ne", "≠"),
    ("neq", "≠"),
    ("infty", "∞"),
    ("propto", "∝"),
    ("approx", "≈"),
    ("equiv", "≡"),
    ("sim", "∼"),
    ("ll", "≪"),
    ("gg", "≫"),
    ("partial", "∂"),
    ("nabla", "∇"),
    ("sum", "∑"),
    ("prod", "∏"),
    ("int", "∫"),
    ("to", "→"),
    ("rightarrow", "→"),
    ("leftarrow", "←"),
    ("leftrightarrow", "↔"),
    ("subset", "⊂"),
    ("subseteq", "⊆"),
    ("cup", "∪"),
    ("cap", "∩"),
    ("emptyset", "∅"),
    ("forall", "∀"),
    ("exists", "∃"),
    ("dots", "…"),
    ("ldots", "…"),
    ("cdots", "…"),
    ("prime", "′"),
    ("dprime", "″"),
    ("degree", "°"),
    ("deg", "°"),
    ("textdegree", "°"),
    ("langle", "⟨"),
    ("rangle", "⟩"),
    ("lceil", "⌈"),
    ("rceil", "⌉"),
    ("lfloor", "⌊"),
    ("rfloor", "⌋"),
    ("textbackslash", "\\"),
    ("textless", "<"),
    ("textgreater", ">"),
    ("space", " "),
];

fn lookup_tex_symbol(name: &str) -> Option<&'static str> {
    TEX_SYMBOLS.iter().find_map(|&(k, v)| (k == name).then_some(v))
}

/// Escape `<`, `>`, `&`, `"`, and `'` for safe HTML text and attribute emission.
pub fn push_html_escaped(raw: &str, out: &mut String) {
    for ch in raw.chars() {
        push_char_escaped(ch, out);
    }
}

/// Return HTML-escaped text as a new `String`.
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
