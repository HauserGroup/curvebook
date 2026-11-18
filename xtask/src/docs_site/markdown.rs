//! Pure-Rust CommonMark/GFM-subset Markdown renderer for the static documentation site.
//!
//! Avoids external parser crates so `xtask` adds zero third-party dependencies while
//! producing deterministic, accessible HTML with GitHub-compatible heading IDs, optional
//! explicit `{#custom-id}` heading anchors, per-page table-of-contents metadata, and
//! rewritten internal/GitHub hyperlinks.

use std::collections::BTreeMap;

/// Heading entry recorded while rendering a Markdown document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TocHeading {
    /// Heading depth (`1..=6`).
    pub level: u8,
    /// Stable HTML `id` attribute (unique within the rendered page).
    pub id: String,
    /// Plain-text heading label stripped of Markdown formatting and `{#...}` suffixes.
    pub text: String,
}

/// Complete HTML and heading output produced from a Markdown source string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedMarkdown {
    /// Rendered HTML fragment for the `<article>` body.
    pub html: String,
    /// Ordered headings emitted in `html`.
    pub headings: Vec<TocHeading>,
    /// Extracted H1 title if the source document opened with `# ...`.
    pub title: Option<String>,
}

/// Link-rewriting strategy invoked on `[text](dest)` and `![alt](src)` targets.
pub trait LinkResolver {
    /// Rewrite a hyperlink target (`href`).
    fn resolve_href(&self, raw_href: &str) -> String;
    /// Rewrite an image asset source (`src`).
    fn resolve_image(&self, raw_src: &str) -> String;
}

/// Identity resolver that leaves links unchanged.
pub struct IdentityResolver;

impl LinkResolver for IdentityResolver {
    fn resolve_href(&self, raw_href: &str) -> String {
        raw_href.to_owned()
    }

    fn resolve_image(&self, raw_src: &str) -> String {
        raw_src.to_owned()
    }
}

/// Render Markdown into safe HTML (no raw `<script>`, `<form>`, or inline `style=`),
/// generating unique heading IDs and resolving links via `resolver`.
#[must_use]
pub fn render_markdown(input: &str, resolver: &dyn LinkResolver) -> RenderedMarkdown {
    let lines: Vec<&str> = input.lines().collect();
    let mut ctx = RenderCtx { resolver, slug_counts: BTreeMap::new(), headings: Vec::new(), title: None };
    let mut html = String::with_capacity(input.len().saturating_mul(2));
    let mut idx = 0usize;

    while idx < lines.len() {
        let line = lines[idx];
        let trimmed = line.trim();
        if trimmed.is_empty() || is_html_comment_start(trimmed) {
            idx = skip_empty_or_comment(&lines, idx);
            continue;
        }
        if let Some((fence_ch, fence_len, lang)) = parse_fence_open(trimmed) {
            idx = render_fenced_code(&lines, idx, fence_ch, fence_len, &lang, &mut html);
            continue;
        }
        if let Some((level, raw_heading)) = parse_atx_heading(trimmed) {
            render_heading_block(&mut ctx, level, raw_heading, &mut html);
            idx += 1;
            continue;
        }
        if is_thematic_break(trimmed) {
            html.push_str("<hr>\n");
            idx += 1;
            continue;
        }
        if looks_like_table(&lines, idx) {
            idx = render_table_block(&mut ctx, &lines, idx, &mut html);
            continue;
        }
        if trimmed.starts_with('>') {
            idx = render_blockquote(&mut ctx, &lines, idx, &mut html);
            continue;
        }
        if let Some((ordered, _, _)) = parse_list_item(line) {
            idx = render_list_block(&mut ctx, &lines, idx, ordered, &mut html);
            continue;
        }
        idx = render_paragraph_block(&mut ctx, &lines, idx, &mut html);
    }

    RenderedMarkdown { html, headings: ctx.headings, title: ctx.title }
}

struct RenderCtx<'a> {
    resolver: &'a dyn LinkResolver,
    slug_counts: BTreeMap<String, usize>,
    headings: Vec<TocHeading>,
    title: Option<String>,
}

impl RenderCtx<'_> {
    fn unique_id(&mut self, base: &str) -> String {
        let norm = if base.is_empty() { String::from("section") } else { base.to_owned() };
        let count = self.slug_counts.entry(norm.clone()).or_insert(0);
        let id = if *count == 0 { norm } else { format!("{norm}-{}", *count) };
        *count += 1;
        id
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

fn is_html_comment_start(trimmed: &str) -> bool {
    trimmed.starts_with("<!--")
}

fn skip_empty_or_comment(lines: &[&str], mut idx: usize) -> usize {
    let trimmed = lines[idx].trim();
    if trimmed.is_empty() {
        return idx + 1;
    }
    while idx < lines.len() {
        if lines[idx].contains("-->") {
            return idx + 1;
        }
        idx += 1;
    }
    idx
}

fn parse_fence_open(trimmed: &str) -> Option<(char, usize, String)> {
    let first = trimmed.chars().next()?;
    if first != '`' && first != '~' {
        return None;
    }
    let count = trimmed.chars().take_while(|&c| c == first).count();
    if count < 3 {
        return None;
    }
    let info = trimmed[count..].trim();
    if first == '`' && info.contains('`') {
        return None;
    }
    let lang = info.split_whitespace().next().unwrap_or("").to_owned();
    Some((first, count, lang))
}

fn is_fence_close(trimmed: &str, ch: char, min_len: usize) -> bool {
    let count = trimmed.chars().take_while(|&c| c == ch).count();
    count >= min_len && trimmed[count..].trim().is_empty()
}

fn render_fenced_code(lines: &[&str], start: usize, ch: char, len: usize, lang: &str, out: &mut String) -> usize {
    let mut idx = start + 1;
    let mut body = String::new();
    while idx < lines.len() {
        if is_fence_close(lines[idx].trim(), ch, len) {
            idx += 1;
            break;
        }
        if !body.is_empty() {
            body.push('\n');
        }
        body.push_str(lines[idx]);
        idx += 1;
    }
    if lang.is_empty() {
        out.push_str("<pre><code>");
    } else {
        let safe_lang: String = lang.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
        if safe_lang.is_empty() {
            out.push_str("<pre><code>");
        } else {
            out.push_str("<pre><code class=\"language-");
            out.push_str(&safe_lang);
            out.push_str("\">");
        }
    }
    push_html_escaped(&body, out);
    out.push_str("</code></pre>\n");
    idx
}

fn parse_atx_heading(trimmed: &str) -> Option<(u8, &str)> {
    let level = trimmed.bytes().take_while(|&b| b == b'#').count();
    if !(1..=6).contains(&level) {
        return None;
    }
    let rest = &trimmed[level..];
    if !rest.is_empty() && !rest.starts_with(' ') && !rest.starts_with('\t') {
        return None;
    }
    let clean = rest.trim().trim_end_matches('#').trim_end();
    Some((level as u8, clean))
}

fn split_heading_custom_id(raw: &str) -> (&str, Option<&str>) {
    let trimmed = raw.trim();
    if let Some(open) = trimmed.rfind("{#")
        && let Some(close) = trimmed[open..].strip_suffix('}')
    {
        let id = close[2..].trim();
        let valid = !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'));
        if valid {
            return (trimmed[..open].trim_end(), Some(id));
        }
    }
    (trimmed, None)
}

fn render_heading_block(ctx: &mut RenderCtx<'_>, level: u8, raw_heading: &str, out: &mut String) {
    let (label_md, custom_id) = split_heading_custom_id(raw_heading);
    let plain = strip_inline_markdown(label_md);
    if level == 1 && ctx.title.is_none() {
        ctx.title = Some(plain.clone());
    }
    let base_slug = match custom_id {
        Some(explicit) => explicit.to_owned(),
        None => github_slug(&plain),
    };
    let id = ctx.unique_id(&base_slug);
    ctx.headings.push(TocHeading { level, id: id.clone(), text: plain });
    let tag_num = level;
    out.push_str(&format!("<h{tag_num} id=\""));
    push_html_escaped(&id, out);
    out.push_str("\" tabindex=\"-1\">");
    render_inline(label_md, ctx.resolver, false, out);
    out.push_str(&format!("</h{tag_num}>\n"));
}

fn is_thematic_break(trimmed: &str) -> bool {
    let chars: Vec<char> = trimmed.chars().filter(|c| !c.is_whitespace()).collect();
    if chars.len() < 3 {
        return false;
    }
    let first = chars[0];
    matches!(first, '-' | '*' | '_') && chars.iter().all(|&c| c == first)
}

fn looks_like_table(lines: &[&str], idx: usize) -> bool {
    let Some(header) = lines.get(idx).map(|s| s.trim()) else {
        return false;
    };
    let Some(delim) = lines.get(idx + 1).map(|s| s.trim()) else {
        return false;
    };
    if !header.contains('|') || !delim.contains('|') {
        return false;
    }
    is_table_delimiter_row(delim)
}

fn is_table_delimiter_row(line: &str) -> bool {
    let cells = split_table_cells(line);
    !cells.is_empty()
        && cells.iter().all(|c| {
            let t = c.trim();
            let core = t.trim_start_matches(':').trim_end_matches(':');
            !core.is_empty() && core.bytes().all(|b| b == b'-')
        })
}

fn split_table_cells(line: &str) -> Vec<&str> {
    let mut s = line.trim();
    if let Some(stripped) = s.strip_prefix('|') {
        s = stripped;
    }
    if let Some(stripped) = s.strip_suffix('|') {
        s = stripped;
    }
    let mut cells = Vec::new();
    let bytes = s.as_bytes();
    let mut start = 0usize;
    let mut i = 0usize;
    let mut in_code = false;
    while i < bytes.len() {
        if bytes[i] >= 0x80 {
            i += 1;
            continue;
        }
        if bytes[i] == b'\\' && i + 1 < bytes.len() && bytes[i + 1] < 0x80 {
            i += 2;
            continue;
        }
        if bytes[i] == b'`' {
            in_code = !in_code;
            i += 1;
            continue;
        }
        if bytes[i] == b'|' && !in_code {
            cells.push(s[start..i].trim());
            start = i + 1;
        }
        i += 1;
    }
    cells.push(s[start..].trim());
    cells
}

fn render_table_block(ctx: &mut RenderCtx<'_>, lines: &[&str], start: usize, out: &mut String) -> usize {
    let headers = split_table_cells(lines[start]);
    out.push_str("<div class=\"table-wrap\"><table>\n<thead><tr>");
    for h in &headers {
        out.push_str("<th>");
        render_inline(h, ctx.resolver, true, out);
        out.push_str("</th>");
    }
    out.push_str("</tr></thead>\n<tbody>\n");
    let mut idx = start + 2;
    while idx < lines.len() {
        let trimmed = lines[idx].trim();
        if trimmed.is_empty() || !trimmed.contains('|') {
            break;
        }
        let row = split_table_cells(trimmed);
        out.push_str("<tr>");
        for col in 0..headers.len() {
            out.push_str("<td>");
            if let Some(cell) = row.get(col) {
                render_inline(cell, ctx.resolver, true, out);
            }
            out.push_str("</td>");
        }
        out.push_str("</tr>\n");
        idx += 1;
    }
    out.push_str("</tbody>\n</table></div>\n");
    idx
}

fn render_blockquote(ctx: &mut RenderCtx<'_>, lines: &[&str], start: usize, out: &mut String) -> usize {
    let mut idx = start;
    let mut inner_lines = Vec::new();
    while idx < lines.len() {
        let trimmed = lines[idx].trim();
        if let Some(rest) = trimmed.strip_prefix('>') {
            inner_lines.push(rest.strip_prefix(' ').unwrap_or(rest));
            idx += 1;
        } else {
            break;
        }
    }
    let joined = inner_lines.join("\n");
    let rendered = render_markdown(&joined, ctx.resolver);
    out.push_str("<blockquote>\n");
    out.push_str(&rendered.html);
    out.push_str("</blockquote>\n");
    idx
}

fn parse_list_item(line: &str) -> Option<(bool, usize, &str)> {
    let indent = line.bytes().take_while(|&b| b == b' ').count();
    let rest = &line[indent..];
    for prefix in ["- ", "* ", "+ "] {
        if let Some(body) = rest.strip_prefix(prefix) {
            return Some((false, indent, body));
        }
    }
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    if digits > 0 {
        let after_num = &rest[digits..];
        if let Some(body) = after_num.strip_prefix(". ").or_else(|| after_num.strip_prefix(") ")) {
            return Some((true, indent, body));
        }
    }
    None
}

fn render_list_block(ctx: &mut RenderCtx<'_>, lines: &[&str], start: usize, ordered: bool, out: &mut String) -> usize {
    let base_indent = lines[start].bytes().take_while(|&b| b == b' ').count();
    let tag = if ordered { "ol" } else { "ul" };
    out.push('<');
    out.push_str(tag);
    out.push_str(">\n");
    let mut idx = start;
    while idx < lines.len() {
        let Some((item_ordered, indent, first_text)) = parse_list_item(lines[idx]) else {
            break;
        };
        if indent < base_indent || (indent == base_indent && item_ordered != ordered) {
            break;
        }
        if indent > base_indent {
            break;
        }
        out.push_str("<li>");
        let mut item_buf = vec![first_text.trim()];
        idx += 1;
        while idx < lines.len() {
            let line = lines[idx];
            let trimmed = line.trim();
            if trimmed.is_empty() {
                if let Some((_, next_indent, _)) = lines.get(idx + 1).and_then(|l| parse_list_item(l))
                    && next_indent <= base_indent
                {
                    idx += 1;
                }
                break;
            }
            if let Some((sub_ord, sub_indent, _)) = parse_list_item(line) {
                if sub_indent > base_indent {
                    if !item_buf.is_empty() {
                        render_inline(&item_buf.join(" "), ctx.resolver, false, out);
                        item_buf.clear();
                    }
                    out.push('\n');
                    idx = render_list_block(ctx, lines, idx, sub_ord, out);
                    continue;
                }
                break;
            }
            if line.starts_with("  ") {
                if let Some((fence_ch, fence_len, lang)) = parse_fence_open(trimmed) {
                    if !item_buf.is_empty() {
                        render_inline(&item_buf.join(" "), ctx.resolver, false, out);
                        item_buf.clear();
                    }
                    out.push('\n');
                    idx = render_fenced_code(lines, idx, fence_ch, fence_len, &lang, out);
                    continue;
                }
                item_buf.push(trimmed);
                idx += 1;
            } else {
                break;
            }
        }
        if !item_buf.is_empty() {
            render_inline(&item_buf.join(" "), ctx.resolver, false, out);
        }
        out.push_str("</li>\n");
    }
    out.push_str("</");
    out.push_str(tag);
    out.push_str(">\n");
    idx
}

fn render_paragraph_block(ctx: &mut RenderCtx<'_>, lines: &[&str], start: usize, out: &mut String) -> usize {
    let mut parts = Vec::new();
    let mut idx = start;
    while idx < lines.len() {
        let trimmed = lines[idx].trim();
        if trimmed.is_empty()
            || is_html_comment_start(trimmed)
            || parse_fence_open(trimmed).is_some()
            || parse_atx_heading(trimmed).is_some()
            || is_thematic_break(trimmed)
            || trimmed.starts_with('>')
            || looks_like_table(lines, idx)
            || parse_list_item(lines[idx]).is_some()
        {
            break;
        }
        parts.push(trimmed);
        idx += 1;
    }
    out.push_str("<p>");
    render_inline(&parts.join(" "), ctx.resolver, false, out);
    out.push_str("</p>\n");
    idx
}

fn render_inline(text: &str, resolver: &dyn LinkResolver, allow_br: bool, out: &mut String) {
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if allow_br && let Some(len) = match_br_tag(&text[i..]) {
            out.push_str("<br>");
            i += len;
            continue;
        }
        if bytes[i] == b'\\' && i + 1 < bytes.len() && bytes[i + 1].is_ascii_punctuation() {
            push_char_escaped(text[i + 1..].chars().next().unwrap_or('\\'), out);
            i += 2;
            continue;
        }
        if bytes[i] == b'`'
            && let Some((code, next_i)) = parse_code_span(&text[i..])
        {
            out.push_str("<code>");
            push_html_escaped(code.trim_matches(' '), out);
            out.push_str("</code>");
            i += next_i;
            continue;
        }
        if bytes[i] == b'!'
            && bytes.get(i + 1) == Some(&b'[')
            && let Some((alt, src, next_i)) = parse_bracket_link(&text[i + 1..])
        {
            let resolved = resolver.resolve_image(src);
            out.push_str("<img src=\"");
            push_html_escaped(&resolved, out);
            out.push_str("\" alt=\"");
            push_html_escaped(alt, out);
            out.push_str("\">");
            i += 1 + next_i;
            continue;
        }
        if bytes[i] == b'['
            && let Some((label, href, next_i)) = parse_bracket_link(&text[i..])
        {
            let resolved = resolver.resolve_href(href);
            out.push_str("<a href=\"");
            push_html_escaped(&resolved, out);
            out.push_str("\">");
            render_inline(label, resolver, allow_br, out);
            out.push_str("</a>");
            i += next_i;
            continue;
        }
        if (text[i..].starts_with("**") || text[i..].starts_with("__"))
            && let Some((inner, next_i)) = parse_delimited_span(&text[i..], 2)
        {
            out.push_str("<strong>");
            render_inline(inner, resolver, allow_br, out);
            out.push_str("</strong>");
            i += next_i;
            continue;
        }
        if text[i..].starts_with("~~")
            && let Some((inner, next_i)) = parse_delimited_span(&text[i..], 2)
        {
            out.push_str("<del>");
            render_inline(inner, resolver, allow_br, out);
            out.push_str("</del>");
            i += next_i;
            continue;
        }
        if (bytes[i] == b'*' || (bytes[i] == b'_' && !is_word_boundary_underscore(bytes, i)))
            && let Some((inner, next_i)) = parse_delimited_span(&text[i..], 1)
        {
            out.push_str("<em>");
            render_inline(inner, resolver, allow_br, out);
            out.push_str("</em>");
            i += next_i;
            continue;
        }
        let ch = text[i..].chars().next().unwrap_or(' ');
        push_char_escaped(ch, out);
        i += ch.len_utf8();
    }
}

fn match_br_tag(slice: &str) -> Option<usize> {
    let b = slice.as_bytes();
    if b.get(..4).is_some_and(|s| s.eq_ignore_ascii_case(b"<br>")) {
        return Some(4);
    }
    if b.get(..5).is_some_and(|s| s.eq_ignore_ascii_case(b"<br/>")) {
        return Some(5);
    }
    if b.get(..6).is_some_and(|s| s.eq_ignore_ascii_case(b"<br />")) {
        return Some(6);
    }
    None
}

fn is_word_boundary_underscore(bytes: &[u8], pos: usize) -> bool {
    let left_alnum = pos > 0 && bytes[pos - 1].is_ascii_alphanumeric();
    let right_alnum = pos + 1 < bytes.len() && bytes[pos + 1].is_ascii_alphanumeric();
    left_alnum && right_alnum
}

fn parse_code_span(slice: &str) -> Option<(&str, usize)> {
    let ticks = slice.bytes().take_while(|&b| b == b'`').count();
    let rest = &slice[ticks..];
    let mut offset = 0usize;
    while let Some(rel) = rest[offset..].find(&"`".repeat(ticks)) {
        let abs = offset + rel;
        let after = abs + ticks;
        let trailing_ticks = rest.as_bytes().get(after) == Some(&b'`');
        let leading_ticks = abs > 0 && rest.as_bytes()[abs - 1] == b'`';
        if !trailing_ticks && !leading_ticks {
            return Some((&rest[..abs], ticks + after));
        }
        offset = abs + 1;
    }
    None
}

fn parse_bracket_link(slice: &str) -> Option<(&str, &str, usize)> {
    let after_open = slice.strip_prefix('[')?;
    let mut depth = 1usize;
    let mut i = 0usize;
    let bytes = after_open.as_bytes();
    let mut in_code = false;
    while i < bytes.len() {
        if bytes[i] >= 0x80 {
            i += 1;
            continue;
        }
        if bytes[i] == b'\\' && i + 1 < bytes.len() && bytes[i + 1] < 0x80 {
            i += 2;
            continue;
        }
        if bytes[i] == b'`' {
            in_code = !in_code;
        } else if !in_code {
            if bytes[i] == b'[' {
                depth += 1;
            } else if bytes[i] == b']' {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
        }
        i += 1;
    }
    if depth != 0 {
        return None;
    }
    let label = &after_open[..i];
    let after_label = after_open[i + 1..].strip_prefix('(')?;
    let mut p_depth = 1usize;
    let mut j = 0usize;
    let hbytes = after_label.as_bytes();
    while j < hbytes.len() {
        if hbytes[j] == b'(' {
            p_depth += 1;
        } else if hbytes[j] == b')' {
            p_depth -= 1;
            if p_depth == 0 {
                let raw_target = after_label[..j].trim();
                let target = raw_target.split_whitespace().next().unwrap_or("").trim_matches('<').trim_matches('>');
                return Some((label, target, 1 + i + 2 + j + 1));
            }
        }
        j += 1;
    }
    None
}

fn parse_delimited_span(slice: &str, width: usize) -> Option<(&str, usize)> {
    let delim = &slice.as_bytes()[..width];
    let ch = delim[0];
    let rest = &slice[width..];
    if rest.is_empty() || rest.starts_with(' ') {
        return None;
    }
    let mut i = 0usize;
    let bytes = rest.as_bytes();
    while i + width <= bytes.len() {
        if bytes[i] >= 0x80 {
            i += 1;
            continue;
        }
        if bytes[i] == b'\\' && i + 1 < bytes.len() && bytes[i + 1] < 0x80 {
            i += 2;
            continue;
        }
        if bytes[i] == b'`'
            && let Some((_, skip)) = parse_code_span(&rest[i..])
        {
            i += skip;
            continue;
        }
        if &bytes[i..i + width] == delim && i > 0 && bytes[i - 1] != b' ' {
            if width == 1 && ch == b'_' && is_word_boundary_underscore(bytes, i) {
                i += 1;
                continue;
            }
            if width == 1 && bytes.get(i + 1) == Some(&ch) {
                i += 2;
                continue;
            }
            return Some((&rest[..i], width + i + width));
        }
        i += 1;
    }
    None
}

/// Strip inline Markdown syntax to plain heading/label text.
#[must_use]
pub fn strip_inline_markdown(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'['
            && let Some((label, _, next_i)) = parse_bracket_link(&input[i..])
        {
            out.push_str(&strip_inline_markdown(label));
            i += next_i;
            continue;
        }
        if matches!(bytes[i], b'*' | b'`' | b'~') {
            i += 1;
            continue;
        }
        let ch = input[i..].chars().next().unwrap_or(' ');
        out.push(ch);
        i += ch.len_utf8();
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
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
