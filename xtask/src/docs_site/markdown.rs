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

const MAX_MARKDOWN_DEPTH: usize = 32;

/// Render Markdown into safe HTML (no raw `<script>`, `<form>`, or inline `style=`),
/// generating unique heading IDs and resolving links via `resolver`.
#[must_use]
pub fn render_markdown(input: &str, resolver: &dyn LinkResolver) -> RenderedMarkdown {
    let lines: Vec<&str> = input.lines().collect();
    let mut ctx = RenderCtx { resolver, slug_counts: BTreeMap::new(), headings: Vec::new(), title: None };
    let mut html = String::with_capacity(input.len().saturating_mul(2));
    render_blocks(&mut ctx, &lines, 0, &mut html);
    RenderedMarkdown { html, headings: ctx.headings, title: ctx.title }
}

fn render_blocks(ctx: &mut RenderCtx<'_>, lines: &[&str], block_depth: usize, html: &mut String) {
    let mut idx = 0usize;

    while idx < lines.len() {
        let line = lines[idx];
        let trimmed = line.trim();
        if trimmed.is_empty() || is_html_comment_start(trimmed) {
            idx = skip_empty_or_comment(lines, idx);
            continue;
        }
        if let Some((fence_ch, fence_len, lang)) = parse_fence_open(trimmed) {
            idx = render_fenced_code(lines, idx, fence_ch, fence_len, &lang, html);
            continue;
        }
        if let Some(next_idx) = try_render_math_block(lines, idx, html) {
            idx = next_idx;
            continue;
        }
        if let Some((level, raw_heading)) = parse_atx_heading(trimmed) {
            render_heading_block(ctx, level, raw_heading, html);
            idx += 1;
            continue;
        }
        if is_thematic_break(trimmed) {
            html.push_str("<hr>\n");
            idx += 1;
            continue;
        }
        if looks_like_table(lines, idx) {
            idx = render_table_block(ctx, lines, idx, html);
            continue;
        }
        if trimmed.starts_with('>') {
            idx = render_blockquote(ctx, lines, idx, block_depth, html);
            continue;
        }
        if let Some((ordered, _, _)) = parse_list_item(line) {
            idx = render_list_block(ctx, lines, idx, ordered, block_depth, html);
            continue;
        }
        idx = render_paragraph_block(ctx, lines, idx, html);
    }
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
    let rest = trimmed.trim_start_matches(first);
    let count = trimmed.len() - rest.len();
    if count < 3 {
        return None;
    }
    let info = rest.trim();
    if first == '`' && info.contains('`') {
        return None;
    }
    let lang = info.split_whitespace().next().unwrap_or("").to_owned();
    Some((first, count, lang))
}

fn is_fence_close(trimmed: &str, ch: char, min_len: usize) -> bool {
    let count = trimmed.chars().take_while(|&c| c == ch).count();
    count >= min_len && trimmed.trim_start_matches(ch).trim().is_empty()
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
    let rest = trimmed.trim_start_matches('#');
    let level = trimmed.len() - rest.len();
    if !(1..=6).contains(&level) {
        return None;
    }
    if !rest.is_empty() && !rest.starts_with(' ') && !rest.starts_with('\t') {
        return None;
    }
    let clean = rest.trim().trim_end_matches('#').trim_end();
    Some((level as u8, clean))
}

fn split_heading_custom_id(raw: &str) -> (&str, Option<&str>) {
    let trimmed = raw.trim();
    if let Some((head, after_open)) = trimmed.rsplit_once("{#")
        && let Some(raw_id) = after_open.strip_suffix('}')
    {
        let id = raw_id.trim();
        let valid = !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'));
        if valid {
            return (head.trim_end(), Some(id));
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
        if !in_code
            && bytes[i] == b'$'
            && let Some(rest) = s.get(i..)
        {
            let skip =
                if rest.starts_with("$$") { parse_block_math_inline(rest).map(|(_, len)| len) } else { parse_inline_math(rest).map(|(_, len)| len) };
            if let Some(len) = skip {
                i += len;
                continue;
            }
        }
        if bytes[i] == b'|' && !in_code {
            cells.push(s.get(start..i).unwrap_or("").trim());
            start = i + 1;
        }
        i += 1;
    }
    cells.push(s.get(start..).unwrap_or("").trim());
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

fn render_blockquote(ctx: &mut RenderCtx<'_>, lines: &[&str], start: usize, block_depth: usize, out: &mut String) -> usize {
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
    if let Some((kind, title, body)) = parse_alert_callout(&inner_lines) {
        out.push_str("<div class=\"callout callout-");
        out.push_str(kind);
        out.push_str("\" role=\"note\">\n<p class=\"callout-title\">");
        out.push_str(title);
        out.push_str("</p>\n");
        if block_depth >= MAX_MARKDOWN_DEPTH {
            out.push_str("<p>");
            push_html_escaped(&body, out);
            out.push_str("</p>\n");
        } else {
            let body_lines: Vec<&str> = body.lines().collect();
            render_blocks(ctx, &body_lines, block_depth + 1, out);
        }
        out.push_str("</div>\n");
        return idx;
    }
    out.push_str("<blockquote>\n");
    if block_depth >= MAX_MARKDOWN_DEPTH {
        out.push_str("<p>");
        push_html_escaped(&inner_lines.join("\n"), out);
        out.push_str("</p>\n");
    } else {
        render_blocks(ctx, &inner_lines, block_depth + 1, out);
    }
    out.push_str("</blockquote>\n");
    idx
}

fn parse_alert_callout(inner_lines: &[&str]) -> Option<(&'static str, &'static str, String)> {
    let first = inner_lines.first()?.trim();
    let markers = [
        ("[!NOTE]", "note", "Note"),
        ("[!TIP]", "tip", "Tip"),
        ("[!IMPORTANT]", "important", "Important"),
        ("[!WARNING]", "warning", "Warning"),
        ("[!CAUTION]", "caution", "Caution"),
    ];
    for (tag, kind, title) in markers {
        if let Some(after) = first.strip_prefix(tag)
            && (after.is_empty() || after.starts_with(char::is_whitespace))
        {
            let mut body_lines: Vec<&str> = Vec::with_capacity(inner_lines.len());
            let first_rest = after.trim();
            if !first_rest.is_empty() {
                body_lines.push(first_rest);
            }
            if let Some(rest) = inner_lines.get(1..) {
                body_lines.extend_from_slice(rest);
            }
            return Some((kind, title, body_lines.join("\n")));
        }
    }
    None
}

fn parse_list_item(line: &str) -> Option<(bool, usize, &str)> {
    let rest = line.trim_start_matches(' ');
    let indent = line.len() - rest.len();
    for prefix in ["- ", "* ", "+ "] {
        if let Some(body) = rest.strip_prefix(prefix) {
            return Some((false, indent, body));
        }
    }
    let after_num = rest.trim_start_matches(|c: char| c.is_ascii_digit());
    let digits = rest.len() - after_num.len();
    if digits > 0
        && let Some(body) = after_num.strip_prefix(". ").or_else(|| after_num.strip_prefix(") "))
    {
        return Some((true, indent, body));
    }
    None
}

fn render_list_block(ctx: &mut RenderCtx<'_>, lines: &[&str], start: usize, ordered: bool, block_depth: usize, out: &mut String) -> usize {
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
                    if block_depth < MAX_MARKDOWN_DEPTH {
                        if !item_buf.is_empty() {
                            render_inline(&item_buf.join(" "), ctx.resolver, false, out);
                            item_buf.clear();
                        }
                        out.push('\n');
                        idx = render_list_block(ctx, lines, idx, sub_ord, block_depth + 1, out);
                        continue;
                    }
                    item_buf.push(trimmed);
                    idx += 1;
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
        if idx > start
            && (trimmed.is_empty()
                || is_html_comment_start(trimmed)
                || parse_fence_open(trimmed).is_some()
                || is_display_math_start(lines, idx)
                || parse_atx_heading(trimmed).is_some()
                || is_thematic_break(trimmed)
                || trimmed.starts_with('>')
                || looks_like_table(lines, idx)
                || parse_list_item(lines[idx]).is_some())
        {
            break;
        }
        parts.push(trimmed);
        idx += 1;
    }
    let joined = parts.join(" ");
    let trimmed_para = joined.trim();
    if let Some(after_bang) = trimmed_para.strip_prefix('!')
        && after_bang.starts_with('[')
        && let Some((alt, src, consumed)) = parse_bracket_link(after_bang)
        && 1 + consumed == trimmed_para.len()
    {
        let resolved = ctx.resolver.resolve_image(src);
        out.push_str("<figure class=\"doc-figure\"><img src=\"");
        push_html_escaped(&resolved, out);
        out.push_str("\" alt=\"");
        push_html_escaped(alt, out);
        out.push_str("\" loading=\"lazy\">");
        let cap = alt.trim();
        if !cap.is_empty() {
            out.push_str("<figcaption>");
            push_html_escaped(cap, out);
            out.push_str("</figcaption>");
        }
        out.push_str("</figure>\n");
        return idx;
    }
    out.push_str("<p>");
    render_inline(&joined, ctx.resolver, false, out);
    out.push_str("</p>\n");
    idx
}

fn render_inline(text: &str, resolver: &dyn LinkResolver, allow_br: bool, out: &mut String) {
    render_inline_depth(text, resolver, allow_br, 0, out);
}

fn render_inline_depth(text: &str, resolver: &dyn LinkResolver, allow_br: bool, depth: usize, out: &mut String) {
    if depth > MAX_MARKDOWN_DEPTH {
        push_html_escaped(text, out);
        return;
    }
    let bytes = text.as_bytes();
    let has_link_mid = text.contains("](");
    // Slots: 0=`**`, 1=`__`, 2=`~~`, 3=`*`, 4=`_`. Once a full scan to EOF fails to find a
    // matching closer for a delimiter kind, later openers of the same kind cannot close either.
    let mut delim_exhausted = [false; 5];
    let mut i = 0usize;
    while let Some(rest) = text.get(i..) {
        if rest.is_empty() {
            break;
        }
        if allow_br && let Some(len) = match_br_tag(rest) {
            out.push_str("<br>");
            i += len;
            continue;
        }
        if bytes[i] == b'\\' && i + 1 < bytes.len() && bytes[i + 1].is_ascii_punctuation() {
            push_char_escaped(char::from(bytes[i + 1]), out);
            i += 2;
            continue;
        }
        if bytes[i] == b'`' {
            if let Some((code, next_i)) = parse_code_span(rest) {
                out.push_str("<code>");
                push_html_escaped(code.trim_matches(' '), out);
                out.push_str("</code>");
                i += next_i;
                continue;
            }
            let ticks = rest.bytes().take_while(|&b| b == b'`').count();
            for _ in 0..ticks {
                out.push('`');
            }
            i += ticks;
            continue;
        }
        if bytes[i] == b'$' {
            if rest.starts_with("$$")
                && let Some((inner, next_i)) = parse_block_math_inline(rest)
            {
                out.push_str("<span class=\"math math-block\">");
                render_tex_math_inner(inner.trim(), 0, out);
                out.push_str("</span>");
                i += next_i;
                continue;
            }
            if let Some((inner, next_i)) = parse_inline_math(rest) {
                out.push_str("<span class=\"math\">");
                render_tex_math_inner(inner, 0, out);
                out.push_str("</span>");
                i += next_i;
                continue;
            }
        }
        if has_link_mid
            && let Some(after_bang) = rest.strip_prefix('!')
            && after_bang.starts_with('[')
            && let Some((alt, src, next_i)) = parse_bracket_link(after_bang)
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
        if has_link_mid
            && bytes[i] == b'['
            && let Some((label, href, next_i)) = parse_bracket_link(rest)
        {
            let resolved = resolver.resolve_href(href);
            out.push_str("<a href=\"");
            push_html_escaped(&resolved, out);
            out.push_str("\">");
            render_inline_depth(label, resolver, allow_br, depth + 1, out);
            out.push_str("</a>");
            i += next_i;
            continue;
        }
        if rest.starts_with("**") || rest.starts_with("__") {
            let slot = if rest.starts_with("**") { 0 } else { 1 };
            if !delim_exhausted[slot] {
                if let Some((inner, next_i)) = parse_delimited_span(rest, 2) {
                    out.push_str("<strong>");
                    render_inline_depth(inner, resolver, allow_br, depth + 1, out);
                    out.push_str("</strong>");
                    i += next_i;
                    continue;
                }
                if rest.get(2..).is_some_and(|r| !r.is_empty() && !r.starts_with(' ')) {
                    delim_exhausted[slot] = true;
                }
            }
        }
        if rest.starts_with("~~") && !delim_exhausted[2] {
            if let Some((inner, next_i)) = parse_delimited_span(rest, 2) {
                out.push_str("<del>");
                render_inline_depth(inner, resolver, allow_br, depth + 1, out);
                out.push_str("</del>");
                i += next_i;
                continue;
            }
            if rest.get(2..).is_some_and(|r| !r.is_empty() && !r.starts_with(' ')) {
                delim_exhausted[2] = true;
            }
        }
        if bytes[i] == b'*' || (bytes[i] == b'_' && !is_word_boundary_underscore(bytes, i)) {
            let slot = if bytes[i] == b'*' { 3 } else { 4 };
            if !delim_exhausted[slot] {
                if let Some((inner, next_i)) = parse_delimited_span(rest, 1) {
                    out.push_str("<em>");
                    render_inline_depth(inner, resolver, allow_br, depth + 1, out);
                    out.push_str("</em>");
                    i += next_i;
                    continue;
                }
                if rest.get(1..).is_some_and(|r| !r.is_empty() && !r.starts_with(' ')) {
                    delim_exhausted[slot] = true;
                }
            }
        }
        let ch = rest.chars().next().unwrap_or(' ');
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
    let rest = slice.trim_start_matches('`');
    let ticks = slice.len() - rest.len();
    if ticks == 0 || ticks > rest.len() {
        return None;
    }
    let needle = "`".repeat(ticks);
    let mut offset = 0usize;
    while let Some(rel) = rest.get(offset..).and_then(|s| s.find(&needle)) {
        let abs = offset + rel;
        let after = abs + ticks;
        let trailing_ticks = rest.as_bytes().get(after) == Some(&b'`');
        let leading_ticks = abs > 0 && rest.as_bytes()[abs - 1] == b'`';
        if !trailing_ticks && !leading_ticks {
            return rest.get(..abs).map(|code| (code, ticks + after));
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
    let (label, after_close_bracket) = after_open.split_at_checked(i)?;
    let after_label = after_close_bracket.strip_prefix("](")?;
    let mut p_depth = 1usize;
    let mut j = 0usize;
    let hbytes = after_label.as_bytes();
    while j < hbytes.len() {
        if hbytes[j] == b'(' {
            p_depth += 1;
        } else if hbytes[j] == b')' {
            p_depth -= 1;
            if p_depth == 0 {
                let raw_target = after_label.get(..j)?.trim();
                let target = raw_target.split_whitespace().next().unwrap_or("").trim_matches('<').trim_matches('>');
                return Some((label, target, 1 + i + 2 + j + 1));
            }
        }
        j += 1;
    }
    None
}

fn parse_delimited_span(slice: &str, width: usize) -> Option<(&str, usize)> {
    let delim = slice.as_bytes().get(..width)?;
    let ch = *delim.first()?;
    let rest = slice.get(width..)?;
    if rest.is_empty() || rest.starts_with(' ') {
        return None;
    }
    let delim_str = std::str::from_utf8(delim).ok()?;
    if !rest.contains(delim_str) {
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
            && let Some(cur) = rest.get(i..)
        {
            if let Some((_, skip)) = parse_code_span(cur) {
                i += skip;
            } else {
                i += cur.bytes().take_while(|&b| b == b'`').count();
            }
            continue;
        }
        if bytes[i] == b'$'
            && let Some(cur) = rest.get(i..)
        {
            let skip =
                if cur.starts_with("$$") { parse_block_math_inline(cur).map(|(_, len)| len) } else { parse_inline_math(cur).map(|(_, len)| len) };
            if let Some(len) = skip {
                i += len;
                continue;
            }
        }
        if bytes.get(i..i + width) == Some(delim) && i > 0 && bytes[i - 1] != b' ' {
            if width == 1 && ch == b'_' && is_word_boundary_underscore(bytes, i) {
                i += 1;
                continue;
            }
            if width == 1 && bytes.get(i + 1) == Some(&ch) {
                i += 2;
                continue;
            }
            return rest.get(..i).map(|inner| (inner, width + i + width));
        }
        i += 1;
    }
    None
}

/// Strip inline Markdown syntax to plain heading/label text.
#[must_use]
pub fn strip_inline_markdown(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    strip_inline_markdown_depth(input, 0, &mut out);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn strip_inline_markdown_depth(input: &str, depth: usize, out: &mut String) {
    if depth > MAX_MARKDOWN_DEPTH {
        out.push_str(input);
        return;
    }
    let bytes = input.as_bytes();
    let has_link_mid = input.contains("](");
    let mut i = 0usize;
    while let Some(rest) = input.get(i..) {
        if rest.is_empty() {
            break;
        }
        if bytes[i] == b'\\' && i + 1 < bytes.len() && bytes[i + 1].is_ascii_punctuation() {
            out.push(char::from(bytes[i + 1]));
            i += 2;
            continue;
        }
        if bytes[i] == b'$' {
            if rest.starts_with("$$")
                && let Some((inner, next_i)) = parse_block_math_inline(rest)
            {
                render_tex_math_plain(inner.trim(), 0, out);
                i += next_i;
                continue;
            }
            if let Some((inner, next_i)) = parse_inline_math(rest) {
                render_tex_math_plain(inner, 0, out);
                i += next_i;
                continue;
            }
        }
        if has_link_mid
            && bytes[i] == b'['
            && let Some((label, _, next_i)) = parse_bracket_link(rest)
        {
            strip_inline_markdown_depth(label, depth + 1, out);
            i += next_i;
            continue;
        }
        if matches!(bytes[i], b'*' | b'`' | b'~') {
            i += 1;
            continue;
        }
        let ch = rest.chars().next().unwrap_or(' ');
        out.push(ch);
        i += ch.len_utf8();
    }
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
    let clean = joined.trim();
    if clean.is_empty() {
        return None;
    }
    Some((clean.to_owned(), close_idx + 1))
}

fn try_render_math_block(lines: &[&str], idx: usize, out: &mut String) -> Option<usize> {
    let (clean, next_idx) = extract_display_math_block(lines, idx)?;
    out.push_str("<div class=\"math-block\"><span class=\"math\">");
    render_tex_math_inner(&clean, 0, out);
    out.push_str("</span></div>\n");
    Some(next_idx)
}

fn parse_block_math_inline(rest: &str) -> Option<(&str, usize)> {
    let after_open = rest.strip_prefix("$$")?;
    let mut i = 0usize;
    let mut brace_depth = 0usize;
    while let Some(cur) = after_open.get(i..) {
        let Some(ch) = cur.chars().next() else {
            break;
        };
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
        if ch == '{' {
            brace_depth = brace_depth.saturating_add(1);
            if brace_depth > MAX_MATH_DEPTH {
                return None;
            }
        } else if ch == '}' {
            if brace_depth == 0 {
                return None;
            }
            brace_depth -= 1;
        } else if brace_depth == 0 && cur.starts_with("$$") {
            let inner = after_open.get(..i)?;
            if inner.trim().is_empty() {
                return None;
            }
            return Some((inner, 2 + i + 2));
        }
        i += ch.len_utf8();
    }
    None
}

fn parse_inline_math(rest: &str) -> Option<(&str, usize)> {
    let after_open = rest.strip_prefix('$')?;
    if after_open.is_empty() || after_open.starts_with('$') {
        return None;
    }
    if after_open.chars().next().is_some_and(char::is_whitespace) {
        return None;
    }
    let mut i = 0usize;
    let mut brace_depth = 0usize;
    while let Some(cur) = after_open.get(i..) {
        let Some(ch) = cur.chars().next() else {
            break;
        };
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
        if ch == '{' {
            brace_depth = brace_depth.saturating_add(1);
            if brace_depth > MAX_MATH_DEPTH {
                return None;
            }
        } else if ch == '}' {
            if brace_depth == 0 {
                return None;
            }
            brace_depth -= 1;
        } else if ch == '$' && brace_depth == 0 {
            let inner = after_open.get(..i)?;
            if inner.is_empty() || inner.chars().last().is_some_and(char::is_whitespace) {
                return None;
            }
            let after_close = after_open.get(i + 1..)?;
            if after_close.starts_with('$') {
                return None;
            }
            if after_close.chars().next().is_some_and(|c| c.is_ascii_alphanumeric() || c == '_') {
                return None;
            }
            return Some((inner, 1 + i + 1));
        }
        i += ch.len_utf8();
    }
    None
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
    if let Some(sym) = lookup_greek_symbol(name).or_else(|| lookup_math_symbol(name)) {
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
            if nlen > 0 {
                let name = after.get(..nlen).unwrap_or("");
                if let Some(sym) = lookup_greek_symbol(name).or_else(|| lookup_math_symbol(name)) {
                    push_html_escaped(sym, out);
                    i += 1 + nlen;
                    continue;
                }
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
            continue;
        }
        if in_tag {
            if ch == '>' {
                in_tag = false;
            }
            continue;
        }
        if ch == '&' {
            in_entity = true;
            continue;
        }
        if in_entity {
            if ch == ';' {
                in_entity = false;
                out.push(accent);
                inserted = true;
            }
            continue;
        }
        if !ch.is_whitespace() {
            out.push(accent);
            inserted = true;
        }
    }
    if !inserted {
        out.push(accent);
    }
}

fn render_tex_math_plain(tex: &str, depth: usize, out: &mut String) {
    if depth > MAX_MATH_DEPTH {
        out.push_str(tex);
        return;
    }
    let mut i = 0usize;
    while let Some(rest) = tex.get(i..) {
        let Some(ch) = rest.chars().next() else {
            break;
        };
        if ch == '{'
            && let Some((inner, consumed)) = extract_braced(rest)
        {
            render_tex_math_plain(inner, depth + 1, out);
            i += consumed;
            continue;
        }
        if ch == '_' || ch == '^' {
            let after = rest.get(1..).unwrap_or("");
            if let Some((arg, consumed)) = extract_math_arg(after, depth) {
                render_tex_math_plain(arg, depth + 1, out);
                i += 1 + consumed;
                continue;
            }
        }
        if ch == '\\' {
            let after = rest.get(1..).unwrap_or("");
            let Some(first) = after.chars().next() else {
                i += 1;
                continue;
            };
            if !first.is_ascii_alphabetic() {
                if !matches!(first, '!' | ',') {
                    out.push(first);
                } else if first == ',' {
                    out.push(' ');
                }
                i += 1 + first.len_utf8();
                continue;
            }
            let nlen = after.bytes().take_while(u8::is_ascii_alphabetic).count();
            let name = after.get(..nlen).unwrap_or("");
            let tail = after.get(nlen..).unwrap_or("");
            if matches!(name, "frac" | "dfrac" | "tfrac")
                && let Some((num, den, consumed)) = extract_two_math_args(tail, depth)
            {
                render_tex_math_plain(num.trim(), depth + 1, out);
                out.push('/');
                render_tex_math_plain(den.trim(), depth + 1, out);
                i += 1 + nlen + consumed;
                continue;
            }
            if name == "sqrt"
                && let Some((_, rad, consumed)) = extract_sqrt_args(tail, depth)
            {
                out.push('\u{221A}');
                render_tex_math_plain(rad.trim(), depth + 1, out);
                i += 1 + nlen + consumed;
                continue;
            }
            if matches!(name, "text" | "mathrm" | "textrm" | "operatorname" | "textbf" | "mathbf" | "textit" | "mathit" | "mbox")
                && let Some((inner, consumed)) = extract_braced_opt_ws(tail)
            {
                out.push_str(inner);
                i += 1 + nlen + consumed;
                continue;
            }
            if matches!(name, "bar" | "overline" | "hat" | "widehat" | "tilde" | "widetilde")
                && let Some((arg, consumed)) = extract_math_arg(tail, depth)
            {
                render_tex_math_plain(arg.trim(), depth + 1, out);
                i += 1 + nlen + consumed;
                continue;
            }
            if matches!(name, "left" | "right") {
                let extra = if tail.starts_with('.') { 1 } else { 0 };
                i += 1 + nlen + extra;
                continue;
            }
            if name == "TeX" {
                out.push_str("TeX");
                i += 1 + nlen;
                continue;
            }
            if name == "LaTeX" {
                out.push_str("LaTeX");
                i += 1 + nlen;
                continue;
            }
            if let Some(sym) = lookup_greek_symbol(name).or_else(|| lookup_math_symbol(name)) {
                out.push_str(sym);
            } else {
                out.push_str(name);
            }
            i += 1 + nlen;
            continue;
        }
        out.push(ch);
        i += ch.len_utf8();
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

fn is_roman_math_function(name: &str) -> bool {
    matches!(
        name,
        "ln" | "log"
            | "exp"
            | "sin"
            | "cos"
            | "tan"
            | "sinh"
            | "cosh"
            | "tanh"
            | "arcsin"
            | "arccos"
            | "arctan"
            | "sec"
            | "csc"
            | "cot"
            | "erf"
            | "erfc"
            | "min"
            | "max"
            | "lim"
            | "sup"
            | "inf"
            | "arg"
            | "det"
            | "dim"
            | "gcd"
            | "lg"
    )
}

fn lookup_greek_symbol(name: &str) -> Option<&'static str> {
    match name {
        "alpha" => Some("\u{03B1}"),
        "beta" => Some("\u{03B2}"),
        "gamma" => Some("\u{03B3}"),
        "delta" => Some("\u{03B4}"),
        "epsilon" | "varepsilon" => Some("\u{03B5}"),
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
        "Omega" => Some("\u{03A9}"),
        _ => None,
    }
}

fn lookup_math_symbol(name: &str) -> Option<&'static str> {
    match name {
        "ge" | "geq" => Some("\u{2265}"),
        "le" | "leq" => Some("\u{2264}"),
        "pm" => Some("\u{00B1}"),
        "mp" => Some("\u{2213}"),
        "times" => Some("\u{00D7}"),
        "div" => Some("\u{00F7}"),
        "cdot" => Some("\u{00B7}"),
        "bullet" => Some("\u{2022}"),
        "in" => Some("\u{2208}"),
        "notin" => Some("\u{2209}"),
        "ne" | "neq" => Some("\u{2260}"),
        "infty" => Some("\u{221E}"),
        "propto" => Some("\u{221D}"),
        "approx" => Some("\u{2248}"),
        "equiv" => Some("\u{2261}"),
        "sim" => Some("\u{223C}"),
        "ll" => Some("\u{226A}"),
        "gg" => Some("\u{226B}"),
        "partial" => Some("\u{2202}"),
        "nabla" => Some("\u{2207}"),
        "sum" => Some("\u{2211}"),
        "prod" => Some("\u{220F}"),
        "int" => Some("\u{222B}"),
        "to" | "rightarrow" => Some("\u{2192}"),
        "leftarrow" => Some("\u{2190}"),
        "leftrightarrow" => Some("\u{2194}"),
        "subset" => Some("\u{2282}"),
        "subseteq" => Some("\u{2286}"),
        "cup" => Some("\u{222A}"),
        "cap" => Some("\u{2229}"),
        "emptyset" => Some("\u{2205}"),
        "forall" => Some("\u{2200}"),
        "exists" => Some("\u{2203}"),
        "dots" | "ldots" | "cdots" => Some("\u{2026}"),
        "prime" => Some("\u{2032}"),
        "dprime" => Some("\u{2033}"),
        "degree" | "deg" | "textdegree" => Some("\u{00B0}"),
        "langle" => Some("\u{27E8}"),
        "rangle" => Some("\u{27E9}"),
        "lceil" => Some("\u{2308}"),
        "rceil" => Some("\u{2309}"),
        "lfloor" => Some("\u{230A}"),
        "rfloor" => Some("\u{230B}"),
        "textbackslash" => Some("\\"),
        "textless" => Some("<"),
        "textgreater" => Some(">"),
        "space" => Some(" "),
        _ => None,
    }
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
