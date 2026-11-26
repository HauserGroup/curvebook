//! `cargo xtask secrets [--history]`: scan tracked files (or git history additions) for API
//! tokens, PEM private keys, inline secret env assignments, and local home-folder paths.

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};

use crate::rust_lex::{find_raw_string_span, skip_quoted_string_end};
use crate::scan_util::{format_error_list, lower_ext};

const PEM_BEGIN_PREFIX: &str = concat!("-----", "BEGIN ");
const PEM_PRIVATE_KEY_SUFFIX: &str = concat!("PRIVATE ", "KEY-----");
const PEM_PGP_PRIVATE_KEY_SUFFIX: &str = concat!("PRIVATE ", "KEY BLOCK-----");
const SECRET_KEY_SUFFIXES: &[&str] =
    &["_API_KEY", "_SECRET_KEY", "_ACCESS_KEY", "_PRIVATE_KEY", "_AUTH_TOKEN", "_SECRET", "_TOKEN", "_PASSWORD", "_PASSWD"];
const EXACT_SECRET_KEYS: &str = "API_KEY SECRET_KEY PRIVATE_KEY AUTH_TOKEN ACCESS_TOKEN PASSWORD SECRET";
const EXACT_PLACEHOLDER_VALUES: &str =
    "example placeholder changeme replace_me replace-me redacted dummy sample not-set not_set unset none null true false empty todo fixme *** xxx";
const PLACEHOLDER_WORDS: &str = "example placeholder changeme replace me your my redacted dummy sample not set unset none null empty todo fixme here value api key secret token password passwd xxx";
const BINARY_EXTS: &str = "png jpg jpeg gif webp bmp ico icns cur pdf zip gz tgz zst xz tar ttf otf woff woff2 eot wasm exe dll so dylib a lib rlib rmeta pdb sqlite db bin wav mp3 mp4 ogg flac curvebook cbtemplate pzf";

fn words_has(list: &str, needle: &str) -> bool {
    !needle.is_empty() && list.split_ascii_whitespace().any(|w| w == needle)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretHit {
    pub location: String,
    pub line: usize,
    pub kind: &'static str,
}

impl std::fmt::Display for SecretHit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}: {}", self.location, self.line, self.kind)
    }
}

pub fn run(root: &Path) -> Result<(), String> {
    let files = crate::repo_files(root)?;
    let hits = scan_repo(root, &files);
    if hits.is_empty() {
        println!("secrets: 0 secret/path findings across {} workspace files (tracked + untracked)", files.len());
        return Ok(());
    }
    Err(format_error_list("potential secret or local home-path exposure(s) found", &hits))
}

pub fn run_cli(root: &Path, args: &[String]) -> Result<(), String> {
    match args {
        [] => run(root),
        [flag] if flag == "--history" => {
            run(root)?;
            run_history(root)
        }
        other => Err(format!("usage: cargo xtask secrets [--history] (unexpected args: `{}`)", other.join(" "))),
    }
}

fn run_history(root: &Path) -> Result<(), String> {
    let mut child = Command::new("git")
        .current_dir(root)
        .args(["-c", "core.quotePath=false", "log", "--all", "-m", "-p", "--text", "--unified=0"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("git log --all -m -p --text --unified=0: {e}"))?;
    let mut reader = BufReader::new(child.stdout.take().ok_or_else(|| String::from("git log stdout pipe missing"))?);
    let (mut state, mut hits, mut buf) = (DiffScanState::new(), Vec::new(), Vec::new());
    loop {
        buf.clear();
        if reader.read_until(b'\n', &mut buf).map_err(|e| format!("read git log output: {e}"))? == 0 {
            break;
        }
        buf.retain(|&b| b != 0);
        let raw = String::from_utf8_lossy(&buf);
        scan_git_diff_line(raw.trim_end_matches(['\r', '\n']), &mut state, &mut hits);
    }
    let output = child.wait_with_output().map_err(|e| format!("wait git log: {e}"))?;
    if !output.status.success() {
        return Err(format!("git log -p --unified=0 failed: {}", String::from_utf8_lossy(&output.stderr)));
    }
    if hits.is_empty() {
        println!("secrets: 0 secret/path findings across git commit history");
        return Ok(());
    }
    Err(format_error_list("potential secret or local home-path exposure(s) found", &hits))
}

#[must_use]
pub fn scan_repo(root: &Path, files: &[String]) -> Vec<SecretHit> {
    let mut hits = Vec::new();
    for path in files {
        let Ok(bytes) = std::fs::read(root.join(path)) else {
            hits.push(SecretHit { location: path.clone(), line: 1, kind: "unreadable file (cannot scan for secrets)" });
            continue;
        };
        if let Ok(Some(text)) = crate::text_decode::decode_utf8_or_utf16(&bytes) {
            scan_text(path, &text, &mut hits);
        } else if !is_likely_binary_file(path, &bytes) {
            let cleaned: Vec<u8> = bytes.iter().copied().filter(|&b| b != 0).collect();
            scan_text(path, &String::from_utf8_lossy(&cleaned), &mut hits);
        }
    }
    hits
}

fn is_likely_binary_file(path: &str, bytes: &[u8]) -> bool {
    const MAGICS: &[&[u8]] = &[
        b"\x89PNG\r\n\x1a\n",
        b"GIF8",
        b"PK\x03\x04",
        b"PK\x05\x06",
        b"%PDF-",
        b"\x00asm",
        b"\x7FELF",
        b"\x1F\x8B",
        b"wOF2",
        b"OTTO",
        b"\x00\x01\x00\x00",
        b"icns",
    ];
    words_has(BINARY_EXTS, &lower_ext(path)) || MAGICS.iter().any(|sig| bytes.starts_with(sig))
}

struct DiffScanState {
    commit: String,
    current_file: String,
    added_line_no: usize,
    remaining_old: usize,
    remaining_new: usize,
    in_hunk: bool,
}

impl DiffScanState {
    fn new() -> Self {
        Self {
            commit: String::from("HEAD"),
            current_file: String::from("<unknown>"),
            added_line_no: 0,
            remaining_old: 0,
            remaining_new: 0,
            in_hunk: false,
        }
    }
}

#[cfg(test)]
#[must_use]
pub fn scan_git_diff(diff: &str) -> Vec<SecretHit> {
    let (mut hits, mut state) = (Vec::new(), DiffScanState::new());
    diff.lines().for_each(|line| scan_git_diff_line(line, &mut state, &mut hits));
    hits
}

fn scan_git_diff_line(raw: &str, st: &mut DiffScanState, hits: &mut Vec<SecretHit>) {
    if st.in_hunk {
        if raw == "\\ No newline at end of file" {
            return;
        }
        if let Some(added) = raw.strip_prefix('+')
            && st.remaining_new > 0
        {
            scan_line(&format!("{}:{}", st.commit, st.current_file), st.added_line_no.max(1), added, hits);
            (st.added_line_no, st.remaining_new) = (st.added_line_no.saturating_add(1), st.remaining_new - 1);
            st.in_hunk = st.remaining_old > 0 || st.remaining_new > 0;
            return;
        }
        if raw.starts_with('-') && st.remaining_old > 0 {
            st.remaining_old -= 1;
            st.in_hunk = st.remaining_old > 0 || st.remaining_new > 0;
            return;
        }
        if raw.starts_with(' ') && st.remaining_old > 0 && st.remaining_new > 0 {
            (st.remaining_old, st.remaining_new, st.added_line_no) = (st.remaining_old - 1, st.remaining_new - 1, st.added_line_no.saturating_add(1));
            st.in_hunk = st.remaining_old > 0 || st.remaining_new > 0;
            return;
        }
        st.in_hunk = false;
    }
    if let Some(rest) = raw.strip_prefix("commit ") {
        (st.commit, st.in_hunk) = (rest.split_whitespace().next().unwrap_or(rest).chars().take(12).collect(), false);
    } else if raw.starts_with("diff --git ") {
        st.in_hunk = false;
    } else if let Some(rest) = raw.strip_prefix("+++ ").map(str::trim_end) {
        st.current_file = if rest == "/dev/null" {
            String::from("<deleted>")
        } else if let Some(u) = rest.strip_prefix("b/") {
            u.to_owned()
        } else if let Some(inner) = rest.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
            let dec = unescape_git_quoted_path(inner);
            dec.strip_prefix("b/").unwrap_or(&dec).to_owned()
        } else {
            rest.to_owned()
        };
    } else if raw.starts_with("--- ") {
    } else if let Some((oc, ns, nc)) = parse_hunk_ranges(raw) {
        (st.added_line_no, st.remaining_old, st.remaining_new, st.in_hunk) = (ns, oc, nc, oc > 0 || nc > 0);
    } else if let Some(added) = raw.strip_prefix('+') {
        scan_line(&format!("{}:{}", st.commit, st.current_file), st.added_line_no.max(1), added, hits);
        st.added_line_no = st.added_line_no.saturating_add(1);
    }
}

fn unescape_git_quoted_path(escaped: &str) -> String {
    let (bytes, mut out, mut i) = (escaped.as_bytes(), Vec::with_capacity(escaped.len()), 0usize);
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            if matches!(bytes[i + 1], b'0'..=b'7')
                && i + 3 < bytes.len()
                && let Ok(oct) = std::str::from_utf8(&bytes[i + 1..i + 4])
                && let Ok(val) = u8::from_str_radix(oct, 8)
            {
                out.push(val);
                i += 4;
                continue;
            }
            out.push(match bytes[i + 1] {
                b'a' => 0x07,
                b'b' => 0x08,
                b't' => b'\t',
                b'n' => b'\n',
                b'v' => 0x0b,
                b'f' => 0x0c,
                b'r' => b'\r',
                o => o,
            });
            i += 2;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn parse_hunk_ranges(line: &str) -> Option<(usize, usize, usize)> {
    let (ranges, _) = line.strip_prefix("@@ ")?.split_once(" @@")?;
    let mut parts = ranges.split_whitespace();
    let oc = parts.next()?.strip_prefix('-')?.split_once(',').map_or(Ok(1), |(_, c)| c.parse()).ok()?;
    let new_part = parts.next()?.strip_prefix('+')?;
    let (ns, nc) = new_part
        .split_once(',')
        .map_or_else(|| Ok((new_part.parse()?, 1)), |(s, c)| Ok::<_, std::num::ParseIntError>((s.parse()?, c.parse()?)))
        .ok()?;
    Some((oc, ns, nc))
}

pub fn scan_text(location: &str, text: &str, out: &mut Vec<SecretHit>) {
    text.lines().enumerate().for_each(|(idx, line)| scan_line(location, idx + 1, line, out));
}

fn scan_line(location: &str, line_no: usize, line: &str, out: &mut Vec<SecretHit>) {
    let mut push = |kind: &'static str| out.push(SecretHit { location: location.to_owned(), line: line_no, kind });
    if contains_pem_private_key(line) {
        push("PEM private key header");
    }
    if let Some(kind) = detect_api_token(line) {
        push(kind);
    }
    if detect_inline_secret_assignment(line) {
        push("inline secret env assignment");
    }
    if let Some(kind) = detect_home_path(line) {
        push(kind);
    }
}

fn contains_pem_private_key(line: &str) -> bool {
    let mut pos = 0usize;
    while let Some(rel) = line.get(pos..).and_then(|s| s.find(PEM_BEGIN_PREFIX)) {
        let after_start = pos + rel + PEM_BEGIN_PREFIX.len();
        let after = line.get(after_start..).unwrap_or("");
        if [PEM_PRIVATE_KEY_SUFFIX, PEM_PGP_PRIVATE_KEY_SUFFIX].iter().any(|sfx| {
            after
                .split_once(sfx)
                .is_some_and(|(k, _)| k.len() <= 40 && k.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || matches!(b, b' ' | b'-')))
        }) {
            return true;
        }
        pos = after_start;
    }
    false
}

fn detect_api_token(line: &str) -> Option<&'static str> {
    let is_dash = |b: u8| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-');
    let is_und = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    if has_prefixed_token(line, "sk-", true, 4, is_dash) {
        Some("API token (`sk-...`)")
    } else if ["ghp_", "gho_", "ghu_", "ghs_", "ghr_"].iter().any(|p| has_prefixed_token(line, p, false, 6, is_und)) {
        Some("GitHub token (`gh*_...`)")
    } else if has_prefixed_token(line, "github_pat_", false, 6, is_und) {
        Some("GitHub fine-grained token (`github_pat_...`)")
    } else if has_aws_access_key(line) {
        Some("AWS access key (`AKIA...`/`ASIA...`)")
    } else if has_prefixed_token(line, "AIza", false, 8, is_dash) {
        Some("Google API key (`AIza...`)")
    } else if ["xoxb-", "xoxa-", "xoxp-", "xoxr-", "xoxs-"].iter().any(|p| has_prefixed_token(line, p, true, 6, is_dash)) {
        Some("Slack token (`xox...`)")
    } else {
        None
    }
}

fn has_prefixed_token(line: &str, prefix: &str, reject_hyphen_left: bool, min_tail: usize, pred: fn(u8) -> bool) -> bool {
    let (bytes, mut start) = (line.as_bytes(), 0usize);
    while let Some(rel) = line.get(start..).and_then(|s| s.find(prefix)) {
        let pos = start + rel;
        let left_ok =
            pos == 0 || (!bytes[pos - 1].is_ascii_alphanumeric() && bytes[pos - 1] != b'_' && (!reject_hyphen_left || bytes[pos - 1] != b'-'));
        if left_ok && bytes[pos + prefix.len()..].iter().take_while(|&&b| pred(b)).count() >= min_tail {
            return true;
        }
        start = pos + 1;
    }
    false
}

fn has_aws_access_key(line: &str) -> bool {
    let bytes = line.as_bytes();
    let is_up = |b: &u8| b.is_ascii_uppercase() || b.is_ascii_digit();
    ["AKIA", "ASIA"].into_iter().any(|prefix| {
        let mut start = 0usize;
        while let Some(rel) = line.get(start..).and_then(|s| s.find(prefix)) {
            let (pos, tail) = (start + rel, start + rel + 4);
            if (pos == 0 || (!bytes[pos - 1].is_ascii_alphanumeric() && bytes[pos - 1] != b'_'))
                && bytes.len() >= tail + 16
                && bytes[tail..tail + 16].iter().all(is_up)
                && bytes.get(tail + 16).is_none_or(|b| !is_up(b))
            {
                return true;
            }
            start = pos + 1;
        }
        false
    })
}

fn detect_inline_secret_assignment(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with("//") {
        return false;
    }
    let rest = trimmed
        .strip_prefix("pub ")
        .or_else(|| trimmed.strip_prefix("pub(").and_then(|s| s.split_once(')')).map(|(_, t)| t.trim_start()))
        .map_or(trimmed, str::trim_start);
    let (is_rust, body) = ["const ", "static mut ", "static ", "let mut ", "let "]
        .iter()
        .find_map(|kw| rest.strip_prefix(kw).map(|a| (true, a.trim_start())))
        .unwrap_or_else(|| (false, rest.strip_prefix("export ").map_or(rest, str::trim_start)));
    let Some((raw_lhs, raw_rhs)) = (if is_rust {
        body.split_once('=')
    } else {
        match (body.find('='), body.find(':')) {
            (Some(e), Some(c)) => body.split_once(if e <= c { '=' } else { ':' }),
            (Some(_), None) => body.split_once('='),
            (None, Some(_)) => body.split_once(':'),
            (None, None) => None,
        }
    }) else {
        return false;
    };
    let lhs = (if is_rust { raw_lhs.split(':').next().unwrap_or(raw_lhs) } else { raw_lhs }).trim().trim_matches(['"', '\'']);
    if lhs.len() < 5
        || !lhs.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
        || !(words_has(EXACT_SECRET_KEYS, lhs) || SECRET_KEY_SUFFIXES.iter().any(|s| lhs.ends_with(s)))
    {
        return false;
    }
    let r = raw_rhs.trim();
    let raw_str = find_raw_string_span(r.as_bytes(), 0).and_then(|(cs, end, h)| {
        let closed = end >= cs + 1 + h && r.as_bytes().get(end - 1 - h) == Some(&b'"') && r.as_bytes()[end - h..end].iter().all(|&b| b == b'#');
        r.get(cs..if closed { end - 1 - h } else { end })
    });
    let dq_off = (r.starts_with("b\"") || r.starts_with("c\"")).then_some(1).or_else(|| r.starts_with('"').then_some(0));
    if is_rust && !(dq_off.is_some() || r.starts_with('\'') || raw_str.is_some()) {
        return false;
    }
    let (val, was_quoted) = if let Some(rb) = raw_str {
        (rb, true)
    } else if let Some(q) = dq_off {
        let end = skip_quoted_string_end(r.as_bytes(), q);
        (r.get(q + 1..end - usize::from(end > q + 1 && r.as_bytes().get(end - 1) == Some(&b'"'))).unwrap_or(""), true)
    } else if let Some(after) = r.strip_prefix('\'') {
        (after.split('\'').next().unwrap_or(""), true)
    } else {
        (r.split('#').next().unwrap_or("").trim().trim_end_matches([',', ';']).trim(), false)
    };
    !is_placeholder_value(val, was_quoted)
}

fn is_placeholder_value(val: &str, was_quoted: bool) -> bool {
    let t = val.trim();
    if t.len() < 6
        || (!was_quoted && (t.contains(char::is_whitespace) || t.contains('(') || t.contains(')') || t.contains("::")))
        || t.starts_with(['$', '<', '{'])
        || t.starts_with("secrets.")
        || t.bytes().all(|b| matches!(b, b'*' | b'x' | b'X' | b'.'))
    {
        return true;
    }
    let lower = t.to_ascii_lowercase();
    if words_has(EXACT_PLACEHOLDER_VALUES, &lower) {
        return true;
    }
    let parts: Vec<&str> = lower.split(|c: char| !c.is_ascii_alphanumeric()).filter(|s| !s.is_empty()).collect();
    !parts.is_empty()
        && (parts.iter().any(|p| matches!(*p, "example" | "placeholder" | "changeme" | "dummy" | "sample" | "redacted"))
            || lower.starts_with("your_")
            || lower.starts_with("your-"))
        && parts.iter().all(|p| words_has(PLACEHOLDER_WORDS, p))
}

fn detect_home_path(line: &str) -> Option<&'static str> {
    if has_unix_home_prefix(line, "/Users/") {
        return Some("local macOS home path (`/Users/<name>/`)");
    }
    if has_unix_home_prefix(line, "/home/") {
        return Some("local Linux home path (`/home/<name>/`)");
    }
    let bytes = line.as_bytes();
    for i in 0..bytes.len().saturating_sub(9) {
        if bytes[i].is_ascii_alphabetic() && bytes[i + 1] == b':' && (i == 0 || !bytes[i - 1].is_ascii_alphanumeric()) {
            let sep_info: Option<(&[u8], usize)> = if bytes.get(i + 2..i + 4) == Some(b"\\\\") {
                Some((b"\\\\", i + 4))
            } else if bytes.get(i + 2) == Some(&b'\\') {
                Some((b"\\", i + 3))
            } else if bytes.get(i + 2) == Some(&b'/') {
                Some((b"/", i + 3))
            } else {
                None
            };
            if let Some((sep, pos)) = sep_info
                && bytes.get(pos..pos + 5).is_some_and(|w| w.eq_ignore_ascii_case(b"users"))
                && let Some(after) = bytes.get(pos + 5..).and_then(|r| r.strip_prefix(sep))
                && matches_non_placeholder_user(after, sep)
            {
                return Some("local Windows home path (`C:\\Users\\<name>\\`)");
            }
        }
    }
    None
}

fn matches_non_placeholder_user(tail: &[u8], sep: &[u8]) -> bool {
    let n = tail.iter().take_while(|&&b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-')).count();
    n > 0
        && tail.get(n..).is_some_and(|r| r.starts_with(sep))
        && !matches!(std::str::from_utf8(&tail[..n]).unwrap_or(""), "YOURNAME" | "YOUR_NAME" | "your-name" | "your_name")
}

fn has_unix_home_prefix(line: &str, prefix: &str) -> bool {
    let (bytes, mut start) = (line.as_bytes(), 0usize);
    while let Some(rel) = line.get(start..).and_then(|s| s.find(prefix)) {
        let pos = start + rel;
        if (pos == 0 || !bytes[pos - 1].is_ascii_alphanumeric()) && matches_non_placeholder_user(&bytes[pos + prefix.len()..], b"/") {
            return true;
        }
        start = pos + 1;
    }
    false
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{run, run_history, scan_git_diff, scan_repo, scan_text};
    use crate::scan_util::test_support::{TestDir, to_utf16_bytes};

    #[test]
    fn catches_tokens_pem_env_home_paths_and_ignores_placeholders_and_self() {
        for sample in [
            concat!("sk-", "test", "1234567890"),
            concat!("ghp_", "AbCdEfGh123456"),
            concat!("github_pat_", "11AABBCCDD"),
            concat!("AKIA", "IOSFODNN7EXAMPLE"),
            concat!("ASIA", "IOSFODNN7EXAMPLE"),
            concat!("AIza", "SyFakeKey12345"),
            concat!("xoxb-", "123456-abcdef"),
            concat!("-----", "BEGIN RSA ", "PRIVATE ", "KEY-----"),
            concat!("-----", "BEGIN PGP ", "PRIVATE ", "KEY BLOCK-----"),
            concat!("OPENAI_API", "_KEY=", "super_secret_token_999"),
            concat!("pub const API_", "SECRET: &[u8] = br#\"prod_sec", "ret_987654\"#;"),
            concat!("/Us", "ers/alice/work/file.txt"),
            concat!("/ho", "me/bob/.ssh/id"),
            concat!("C:\\\\Us", "ers\\\\carol\\\\Documents\\\\x"),
        ] {
            let mut hits = Vec::new();
            scan_text("probe.rs", sample, &mut hits);
            assert!(!hits.is_empty(), "missed `{sample}`");
        }
        let safe =
            "disk-full\nGITHUB_TOKEN=${{ github.token }}\nAPI_KEY=changeme\n/Users/YOURNAME/Documents/CurveBook\nlet API_KEY = env::var(\"X\");\n";
        let (mut safe_hits, mut own_hits) = (Vec::new(), Vec::new());
        scan_text("safe.txt", safe, &mut safe_hits);
        assert!(safe_hits.is_empty(), "{safe_hits:?}");
        let own = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/secrets.rs")).unwrap();
        scan_text("xtask/src/secrets.rs", &own, &mut own_hits);
        assert!(own_hits.is_empty(), "{own_hits:?}");
    }

    #[test]
    fn scans_diff_utf16_missing_and_merge_history() {
        let sk = concat!("sk-", "test", "abcdef12");
        let diff = format!("commit 1234567890ab\n+++ \"b/src/caf\\303\\251/cfg.rs\"\n@@ -1,0 +1,2 @@\n+++ fake\n+let key = \"{sk}\";\n");
        let dh = scan_git_diff(&diff);
        assert_eq!((dh.len(), dh[0].location.as_str(), dh[0].line), (1, "1234567890ab:src/café/cfg.rs", 2));

        let dir = TestDir::new("secrets-repo");
        dir.write_bytes("u16.txt", &to_utf16_bytes(&format!("key = \"{sk}\"\n"), true, false));
        assert_eq!(scan_repo(dir.path(), &[String::from("u16.txt"), String::from("missing.txt")]).len(), 2);

        let repo = TestDir::git_repo("secrets-hist");
        for (body, msg) in [("base\n", "base"), (&format!("key = \"{sk}\"\n") as &str, "leak"), ("clean\n", "clean")] {
            repo.write("cfg.txt", body);
            repo.git_add_all();
            repo.run_git(&["commit", "-q", "-m", msg]);
        }
        assert!(run(repo.path()).is_ok() && run_history(repo.path()).unwrap_err().contains("cfg.txt"));
    }
}
