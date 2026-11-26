//! Shared minimal SFNT (TrueType `glyf` and OpenType `CFF `) builders for `text_suite`.

pub fn push_u16(buf: &mut Vec<u8>, v: u16) {
    buf.extend_from_slice(&v.to_be_bytes());
}

pub fn push_i16(buf: &mut Vec<u8>, v: i16) {
    buf.extend_from_slice(&v.to_be_bytes());
}

pub fn push_u32(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_be_bytes());
}

pub fn assemble_sfnt(sfnt_version: u32, mut tables: Vec<([u8; 4], Vec<u8>)>) -> Vec<u8> {
    tables.sort_by_key(|(tag, _)| *tag);
    let dir_size = 12 + tables.len() * 16;
    let mut font = Vec::new();
    push_u32(&mut font, sfnt_version);
    push_u16(&mut font, u16::try_from(tables.len()).expect("table count"));
    push_u16(&mut font, 128);
    push_u16(&mut font, 3);
    push_u16(&mut font, 0);

    let mut offset = dir_size;
    for (tag, data) in &tables {
        font.extend_from_slice(tag);
        push_u32(&mut font, 0);
        push_u32(&mut font, u32::try_from(offset).expect("table offset"));
        push_u32(&mut font, u32::try_from(data.len()).expect("table len"));
        offset += (data.len() + 3) & !3;
    }
    for (_, data) in &tables {
        font.extend_from_slice(data);
        while !font.len().is_multiple_of(4) {
            font.push(0);
        }
    }
    font
}

pub fn build_head(loca_fmt: i16) -> Vec<u8> {
    let mut head = vec![0u8; 54];
    head[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    head[12..16].copy_from_slice(&0x5F0F_3CF5u32.to_be_bytes());
    head[18..20].copy_from_slice(&1000u16.to_be_bytes());
    head[50..52].copy_from_slice(&loca_fmt.to_be_bytes());
    head
}

pub fn build_hhea(asc: i16, desc: i16, line_gap: i16, n_hmetrics: u16) -> Vec<u8> {
    let mut hhea = vec![0u8; 36];
    hhea[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    hhea[4..6].copy_from_slice(&asc.to_be_bytes());
    hhea[6..8].copy_from_slice(&desc.to_be_bytes());
    hhea[8..10].copy_from_slice(&line_gap.to_be_bytes());
    hhea[34..36].copy_from_slice(&n_hmetrics.to_be_bytes());
    hhea
}

pub fn build_maxp_v05(num_glyphs: u16) -> Vec<u8> {
    let mut maxp = vec![0u8; 6];
    maxp[0..4].copy_from_slice(&0x0000_5000u32.to_be_bytes());
    maxp[4..6].copy_from_slice(&num_glyphs.to_be_bytes());
    maxp
}

pub fn build_maxp_v10(num_glyphs: u16, max_points: u16, max_contours: u16) -> Vec<u8> {
    let mut maxp = vec![0u8; 32];
    maxp[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    maxp[4..6].copy_from_slice(&num_glyphs.to_be_bytes());
    maxp[6..8].copy_from_slice(&max_points.to_be_bytes());
    maxp[8..10].copy_from_slice(&max_contours.to_be_bytes());
    maxp
}

pub fn build_post_v3() -> Vec<u8> {
    let mut post = vec![0u8; 32];
    post[0..4].copy_from_slice(&0x0003_0000u32.to_be_bytes());
    post
}

pub fn build_name_table(family: &str, postscript: &str) -> Vec<u8> {
    let mut str_buf = Vec::new();
    for u in family.encode_utf16() {
        push_u16(&mut str_buf, u);
    }
    let fam_len = u16::try_from(str_buf.len()).expect("family len");
    for u in postscript.encode_utf16() {
        push_u16(&mut str_buf, u);
    }
    let ps_len = u16::try_from(str_buf.len()).expect("ps len") - fam_len;

    let mut name = Vec::new();
    push_u16(&mut name, 0);
    push_u16(&mut name, 2);
    push_u16(&mut name, 6 + 2 * 12);
    for (name_id, len, off) in [(1u16, fam_len, 0u16), (6u16, ps_len, fam_len)] {
        for v in [3u16, 1, 0x0409, name_id, len, off] {
            push_u16(&mut name, v);
        }
    }
    name.extend_from_slice(&str_buf);
    name
}

pub fn build_cmap4_subtable(segments: &[(u16, u16, i16, u16)], extra_gids: &[u16]) -> Vec<u8> {
    let seg_count = u16::try_from(segments.len()).expect("seg count");
    let sub_len = u16::try_from(16 + segments.len() * 8 + extra_gids.len() * 2).expect("cmap4 len");
    let mut sub = Vec::new();
    for v in [4u16, sub_len, 0, seg_count * 2, 0, 0, 0] {
        push_u16(&mut sub, v);
    }
    for &(_, end, _, _) in segments {
        push_u16(&mut sub, end);
    }
    push_u16(&mut sub, 0);
    for &(start, _, _, _) in segments {
        push_u16(&mut sub, start);
    }
    for &(_, _, delta, _) in segments {
        push_i16(&mut sub, delta);
    }
    for &(_, _, _, range_off) in segments {
        push_u16(&mut sub, range_off);
    }
    for &gid in extra_gids {
        push_u16(&mut sub, gid);
    }
    sub
}

pub fn build_single_bmp_cmap4(codepoint: u16, gid: u16) -> Vec<u8> {
    let delta = i16::from_ne_bytes(u16::try_from((i32::from(gid) - i32::from(codepoint)).rem_euclid(65_536)).expect("delta").to_ne_bytes());
    build_cmap4_subtable(&[(codepoint, codepoint, delta, 0), (0xFFFF, 0xFFFF, 1, 0)], &[])
}

pub fn build_cmap12_subtable(groups: &[(u32, u32, u32)]) -> Vec<u8> {
    let mut sub = Vec::new();
    push_u16(&mut sub, 12);
    push_u16(&mut sub, 0);
    push_u32(&mut sub, u32::try_from(16 + groups.len() * 12).expect("cmap12 len"));
    push_u32(&mut sub, 0);
    push_u32(&mut sub, u32::try_from(groups.len()).expect("cmap12 groups"));
    for &(start, end, start_gid) in groups {
        push_u32(&mut sub, start);
        push_u32(&mut sub, end);
        push_u32(&mut sub, start_gid);
    }
    sub
}

pub fn wrap_cmap_subtables(entries: &[(u16, u16, Vec<u8>)]) -> Vec<u8> {
    let mut cmap = Vec::new();
    push_u16(&mut cmap, 0);
    push_u16(&mut cmap, u16::try_from(entries.len()).expect("n subtables"));
    let mut off = 4 + entries.len() * 8;
    for (plat, enc, data) in entries {
        push_u16(&mut cmap, *plat);
        push_u16(&mut cmap, *enc);
        push_u32(&mut cmap, u32::try_from(off).expect("cmap sub off"));
        off += data.len();
    }
    for (_, _, data) in entries {
        cmap.extend_from_slice(data);
    }
    cmap
}

/// Build a 2-glyph SFNT `glyf` font mapping `codepoint -> gid 1` with `contours` triangles (0 = empty glyph).
pub fn build_glyf_font(family: &str, postscript: &str, descender: i16, codepoint: u16, contours: u16) -> Vec<u8> {
    let total_points = u16::try_from(usize::from(contours) * 3).expect("points fit u16");
    let mut hmtx = Vec::with_capacity(8);
    for (adv, lsb) in [(500u16, 0i16), (600, 20)] {
        push_u16(&mut hmtx, adv);
        push_i16(&mut hmtx, lsb);
    }
    let (glyf, loca) = if contours == 0 {
        (vec![0u8; 4], vec![0u8; 6])
    } else {
        let mut g = Vec::new();
        for v in [i16::try_from(contours).expect("contours"), 10, 0, 600, 700] {
            push_i16(&mut g, v);
        }
        for c in 0..contours {
            push_u16(&mut g, c * 3 + 2);
        }
        push_u16(&mut g, 0);
        g.resize(g.len() + usize::from(total_points), 0x37);
        for c in 0..contours {
            g.extend_from_slice(&[if c == 0 { 10 } else { 1 }, 20, 5]);
        }
        for c in 0..contours {
            g.extend_from_slice(&[if c == 0 { 10 } else { 1 }, 0, 25]);
        }
        while !g.len().is_multiple_of(4) {
            g.push(0);
        }
        let mut l = Vec::new();
        for v in [0u16, 0, u16::try_from(g.len() / 2).expect("short loca")] {
            push_u16(&mut l, v);
        }
        (g, l)
    };
    assemble_sfnt(
        0x0001_0000,
        vec![
            (*b"cmap", wrap_cmap_subtables(&[(3, 1, build_single_bmp_cmap4(codepoint, 1))])),
            (*b"glyf", glyf),
            (*b"head", build_head(0)),
            (*b"hhea", build_hhea(800, descender, 90, 2)),
            (*b"hmtx", hmtx),
            (*b"loca", loca),
            (*b"maxp", build_maxp_v10(2, total_points.max(3), contours.max(1))),
            (*b"name", build_name_table(family, postscript)),
            (*b"post", build_post_v3()),
        ],
    )
}

pub fn cff_push_int(buf: &mut Vec<u8>, v: i32) {
    if (-107..=107).contains(&v) {
        buf.push(u8::try_from(v + 139).expect("cff int1"));
    } else if (108..=1131).contains(&v) {
        let d = v - 108;
        buf.push(u8::try_from((d / 256) + 247).expect("cff int2+"));
        buf.push(u8::try_from(d % 256).expect("cff int2+ lo"));
    } else if (-1131..=-108).contains(&v) {
        let d = -v - 108;
        buf.push(u8::try_from((d / 256) + 251).expect("cff int2-"));
        buf.push(u8::try_from(d % 256).expect("cff int2- lo"));
    } else {
        buf.push(29);
        buf.extend_from_slice(&v.to_be_bytes());
    }
}

fn cff_encode_index(items: &[&[u8]]) -> Vec<u8> {
    let mut out = Vec::new();
    push_u16(&mut out, u16::try_from(items.len()).expect("cff index count"));
    if items.is_empty() {
        return out;
    }
    let total: usize = items.iter().map(|s| s.len()).sum();
    let off_size: u8 = if total < 255 {
        1
    } else if total < 65_535 {
        2
    } else {
        4
    };
    out.push(off_size);
    let mut offset = 1u32;
    let push_off = |buf: &mut Vec<u8>, v: u32| match off_size {
        1 => buf.push(u8::try_from(v).expect("u8 off")),
        2 => push_u16(buf, u16::try_from(v).expect("u16 off")),
        _ => push_u32(buf, v),
    };
    push_off(&mut out, offset);
    for item in items {
        offset += u32::try_from(item.len()).expect("item len");
        push_off(&mut out, offset);
    }
    for item in items {
        out.extend_from_slice(item);
    }
    out
}

/// Build a minimal valid OpenType CFF (`OTTO`) font mapping `'A'` (`0x0041`) to `gid 1`.
pub fn build_synthetic_cff_otf(charstring_a: &[u8], global_subrs: &[&[u8]]) -> Vec<u8> {
    let name_index = cff_encode_index(&[b"SynthCFF"]);
    let string_index = cff_encode_index(&[]);
    let gsubr_index = cff_encode_index(global_subrs);
    let charset = [0u8, 0, 34];
    let notdef_cs = [14u8];
    let charstrings_index = cff_encode_index(&[&notdef_cs, charstring_a]);
    let private_dict = [139u8, 20];

    let encode_top = |cs_set_off: i32, cs_off: i32, priv_len: i32, priv_off: i32| {
        let mut d = Vec::new();
        for (val, op) in [(cs_set_off, Some(15u8)), (cs_off, Some(17)), (priv_len, None), (priv_off, Some(18))] {
            d.push(29);
            d.extend_from_slice(&val.to_be_bytes());
            if let Some(op) = op {
                d.push(op);
            }
        }
        d
    };
    let top_len = cff_encode_index(&[&encode_top(0, 0, 0, 0)]).len();
    let charset_off = 4 + name_index.len() + top_len + string_index.len() + gsubr_index.len();
    let cs_off = charset_off + charset.len();
    let priv_off = cs_off + charstrings_index.len();
    let top_index = cff_encode_index(&[&encode_top(
        i32::try_from(charset_off).expect("charset off"),
        i32::try_from(cs_off).expect("cs off"),
        i32::try_from(private_dict.len()).expect("priv len"),
        i32::try_from(priv_off).expect("priv off"),
    )]);
    assert_eq!(top_index.len(), top_len);

    let mut cff = vec![1, 0, 4, 4];
    for chunk in [&name_index[..], &top_index, &string_index, &gsubr_index, &charset, &charstrings_index, &private_dict] {
        cff.extend_from_slice(chunk);
    }
    let mut hmtx = Vec::new();
    for adv in [500u16, 600] {
        push_u16(&mut hmtx, adv);
        push_i16(&mut hmtx, 0);
    }
    assemble_sfnt(
        u32::from_be_bytes(*b"OTTO"),
        vec![
            (*b"CFF ", cff),
            (*b"cmap", wrap_cmap_subtables(&[(3, 1, build_single_bmp_cmap4(0x0041, 1))])),
            (*b"head", build_head(0)),
            (*b"hhea", build_hhea(800, -200, 0, 2)),
            (*b"hmtx", hmtx),
            (*b"maxp", build_maxp_v05(2)),
            (*b"name", build_name_table("SynthCFF", "SynthCFF")),
            (*b"post", build_post_v3()),
        ],
    )
}
