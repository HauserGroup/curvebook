#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::io::Read;

use flate2::read::ZlibDecoder;

mod audit_p4_sec10;
mod audit_r1_s4b_s09;
mod external_viewer_qa;
mod pdf17_truetype_embed;
mod r1019_format_determinism;
mod spike_s2_pdf_fidelity;

pub(crate) struct ParsedPdf {
    pub(crate) objects: BTreeMap<usize, Vec<u8>>,
    pub(crate) xref_offsets: Vec<(usize, usize, char)>,
    pub(crate) trailer: String,
    pub(crate) streams: Vec<(usize, String, Vec<u8>)>,
}

pub(crate) fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

pub(crate) fn rfind_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).rposition(|w| w == needle)
}

pub(crate) fn extract_pdf_int(dict: &str, key: &str) -> Option<usize> {
    let (_, rest) = dict.split_once(key)?;
    rest.chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().ok()
}

pub(crate) fn parse_pdf_strict(bytes: &[u8], expected_headers: &[&[u8]], require_binary_comment: bool) -> Result<ParsedPdf, String> {
    if !expected_headers.iter().any(|h| bytes.starts_with(h)) {
        return Err(format!("missing expected PDF header among {expected_headers:?}"));
    }
    if require_binary_comment && !bytes.windows(6).any(|w| w == b"%\xE2\xE3\xCF\xD3\n") {
        return Err("missing PDF binary comment %âãÏÓ".to_owned());
    }
    if !bytes.ends_with(b"%%EOF\n") {
        return Err("missing %%EOF\\n footer".to_owned());
    }

    let startxref_pos = rfind_subslice(bytes, b"startxref\n").ok_or("missing startxref")?;
    let after = std::str::from_utf8(bytes.get(startxref_pos + b"startxref\n".len()..).ok_or("startxref tail")?)
        .map_err(|e| format!("ASCII startxref: {e}"))?;
    let xref_offset: usize = after.lines().next().ok_or("missing xref offset line")?.trim().parse().map_err(|e| format!("xref offset: {e}"))?;
    if xref_offset >= startxref_pos || !bytes.get(xref_offset..).is_some_and(|s| s.starts_with(b"xref\n")) {
        return Err("invalid xref offset".to_owned());
    }

    let xref_to_start = bytes.get(xref_offset..startxref_pos).ok_or("xref slice")?;
    let trailer_rel = find_subslice(xref_to_start, b"trailer\n").ok_or("missing trailer after xref")?;
    let xref_str = std::str::from_utf8(xref_to_start.get(..trailer_rel).ok_or("xref utf8")?).map_err(|e| format!("xref utf8: {e}"))?;
    let trailer = std::str::from_utf8(xref_to_start.get(trailer_rel..).ok_or("trailer utf8")?).map_err(|e| format!("trailer utf8: {e}"))?.to_owned();

    let mut lines = xref_str.lines();
    if lines.next() != Some("xref") {
        return Err("missing xref header line".to_owned());
    }
    let count_parts: Vec<&str> = lines.next().ok_or("missing xref range line")?.split_whitespace().collect();
    let ["0", count_str] = count_parts.as_slice() else {
        return Err(format!("unexpected xref range: {count_parts:?}"));
    };
    let total_entries: usize = count_str.parse().map_err(|e| format!("xref total: {e}"))?;

    let mut xref_offsets = Vec::with_capacity(total_entries);
    for (idx, line) in lines.enumerate() {
        if line.len() != 19 {
            return Err(format!("xref entry must be 20 bytes including LF: `{line}`"));
        }
        let off: usize = line.get(0..10).ok_or("xref off")?.parse().map_err(|e| format!("xref off: {e}"))?;
        let generation: usize = line.get(11..16).ok_or("xref gen")?.parse().map_err(|e| format!("xref gen: {e}"))?;
        let kind = line.chars().nth(17).ok_or("xref kind")?;
        if idx == 0 {
            if (off, generation, kind) != (0, 65535, 'f') {
                return Err(format!("bad xref entry 0: ({off}, {generation}, {kind})"));
            }
        } else if (generation, kind) != (0, 'n') || off >= xref_offset {
            return Err(format!("bad xref entry {idx}: ({off}, {generation}, {kind})"));
        }
        xref_offsets.push((off, generation, kind));
    }
    if xref_offsets.len() != total_entries {
        return Err(format!("xref count mismatch: {} vs {total_entries}", xref_offsets.len()));
    }

    let mut objects = BTreeMap::new();
    let mut streams = Vec::new();
    for (obj_id, &(off, _, kind)) in xref_offsets.iter().enumerate() {
        if kind != 'n' {
            continue;
        }
        let prefix = format!("{obj_id} 0 obj\n");
        let obj_tail = bytes.get(off..xref_offset).ok_or("object range")?;
        if !obj_tail.starts_with(prefix.as_bytes()) {
            return Err(format!("missing object header {prefix} at offset {off}"));
        }
        let after_prefix = obj_tail.get(prefix.len()..).ok_or("object body range")?;
        let endobj_rel = find_subslice(after_prefix, b"endobj\n").ok_or("missing endobj\\n")?;
        let body = after_prefix.get(..endobj_rel).ok_or("object body slice")?;
        objects.insert(obj_id, body.to_vec());

        if let Some(stream_rel) = find_subslice(body, b"\nstream\n") {
            let dict_str = String::from_utf8_lossy(body.get(..stream_rel).ok_or("dict slice")?).into_owned();
            let stream_tail = body.get(stream_rel + b"\nstream\n".len()..).ok_or("stream tail")?;
            let endstream_rel = find_subslice(stream_tail, b"\nendstream").ok_or("missing \\nendstream")?;
            let raw_stream = stream_tail.get(..endstream_rel).ok_or("raw stream slice")?;
            let declared_len = extract_pdf_int(&dict_str, "/Length ").ok_or("missing /Length")?;
            if raw_stream.len() != declared_len {
                return Err(format!("object {obj_id} /Length mismatch: {} vs {declared_len}", raw_stream.len()));
            }
            let decoded = if dict_str.contains("/Filter /FlateDecode") {
                let mut dec = ZlibDecoder::new(raw_stream);
                let mut out = Vec::new();
                dec.read_to_end(&mut out).map_err(|e| format!("zlib FlateDecode: {e}"))?;
                out
            } else {
                raw_stream.to_vec()
            };
            streams.push((obj_id, dict_str, decoded));
        }
    }

    Ok(ParsedPdf { objects, xref_offsets, trailer, streams })
}
