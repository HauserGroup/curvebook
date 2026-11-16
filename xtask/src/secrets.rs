//! `cargo xtask secrets [--history]`: scan tracked files (or git history additions) for API
//! tokens, PEM private keys, inline secret env assignments, and local home-folder paths.

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};

const PEM_BEGIN_PREFIX: &str = concat!("-----", "BEGIN ");
const PEM_PRIVATE_KEY_SUFFIX: &str = concat!("PRIVATE ", "KEY-----");

const SECRET_KEY_SUFFIXES: &[&str] =
    &["_API_KEY", "_SECRET_KEY", "_ACCESS_KEY", "_PRIVATE_KEY", "_AUTH_TOKEN", "_SECRET", "_TOKEN", "_PASSWORD", "_PASSWD"];
const EXACT_SECRET_KEYS: &[&str] = &["API_KEY", "SECRET_KEY", "PRIVATE_KEY", "AUTH_TOKEN", "ACCESS_TOKEN", "PASSWORD", "SECRET"];

const EXACT_PLACEHOLDER_VALUES: &[&str] = &[
    "example",
    "placeholder",
    "changeme",
    "replace_me",
    "replace-me",
    "redacted",
    "dummy",
    "sample",
    "not-set",
    "not_set",
    "unset",
    "none",
    "null",
    "true",
    "false",
    "empty",
    "todo",
    "fixme",
    "***",
    "xxx",
];

const PLACEHOLDER_WORDS: &[&str] = &[
    "example",
    "placeholder",
    "changeme",
    "replace",
    "me",
    "your",
    "my",
    "redacted",
    "dummy",
    "sample",
    "not",
    "set",
    "unset",
    "none",
    "null",
    "empty",
    "todo",
    "fixme",
    "here",
    "value",
    "api",
    "key",
    "secret",
    "token",
    "password",
    "passwd",
    "xxx",
];

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

/// Run the working-tree secrets scan (used by `cargo xtask ci` and `cargo xtask secrets`).
pub fn run(root: &Path) -> Result<(), String> {
    let files = crate::repo_files(root)?;
    let hits = scan_repo(root, &files);
    if hits.is_empty() {
        println!("secrets: 0 secret/path findings across {} tracked files", files.len());
        return Ok(());
    }
    format_err(&hits)
}

/// CLI entrypoint for `cargo xtask secrets [--history]`.
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
        .args(["-c", "core.quotePath=false", "log", "--all", "-p", "--text", "--unified=0"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("git log --all -p --text --unified=0: {e}"))?;

    let stdout = child.stdout.take().ok_or_else(|| String::from("git log stdout pipe missing"))?;
    let mut reader = BufReader::new(stdout);
    let mut state = DiffScanState::new();
    let mut hits = Vec::new();
    let mut buf = Vec::new();
    loop {
        buf.clear();
        let n = reader.read_until(b'\n', &mut buf).map_err(|e| format!("read git log output: {e}"))?;
        if n == 0 {
            break;
        }
        if buf.contains(&0) {
            continue;
        }
        let raw_line = String::from_utf8_lossy(&buf);
        let line = raw_line.trim_end_matches(['\r', '\n']);
        scan_git_diff_line(line, &mut state, &mut hits);
    }

    let output = child.wait_with_output().map_err(|e| format!("wait git log: {e}"))?;
    if !output.status.success() {
        return Err(format!("git log -p --unified=0 failed: {}", String::from_utf8_lossy(&output.stderr)));
    }
    if hits.is_empty() {
        println!("secrets: 0 secret/path findings across git commit history");
        return Ok(());
    }
    format_err(&hits)
}

fn format_err(hits: &[SecretHit]) -> Result<(), String> {
    let lines: Vec<String> = hits.iter().map(ToString::to_string).collect();
    Err(format!("{} potential secret or local home-path exposure(s) found:\n  {}", hits.len(), lines.join("\n  ")))
}

/// Scan `files` relative to `root`, skipping binary blobs containing NUL bytes.
#[must_use]
pub fn scan_repo(root: &Path, files: &[String]) -> Vec<SecretHit> {
    let mut hits = Vec::new();
    for path in files {
        let Ok(bytes) = std::fs::read(root.join(path)) else {
            continue;
        };
        if bytes.contains(&0) {
            continue;
        }
        let text = String::from_utf8_lossy(&bytes);
        scan_text(path, &text, &mut hits);
    }
    hits
}

struct DiffScanState {
    commit: String,
    current_file: String,
    added_line_no: usize,
}

impl DiffScanState {
    fn new() -> Self {
        Self { commit: String::from("HEAD"), current_file: String::from("<unknown>"), added_line_no: 0 }
    }
}

/// Scan added (`+`) lines in `git log -p --unified=0` output.
#[cfg(test)]
#[must_use]
pub fn scan_git_diff(diff: &str) -> Vec<SecretHit> {
    let mut hits = Vec::new();
    let mut state = DiffScanState::new();
    for raw_line in diff.lines() {
        scan_git_diff_line(raw_line, &mut state, &mut hits);
    }
    hits
}

fn scan_git_diff_line(raw_line: &str, state: &mut DiffScanState, hits: &mut Vec<SecretHit>) {
    if let Some(rest) = raw_line.strip_prefix("commit ") {
        let short = rest.split_whitespace().next().unwrap_or(rest);
        state.commit = short.chars().take(12).collect();
        return;
    }
    if raw_line.starts_with("+++ ") {
        if let Some(path) = parse_diff_plus_path(raw_line) {
            state.current_file = path;
        }
        return;
    }
    if raw_line.starts_with("--- ") {
        return;
    }
    if let Some(plus_line) = parse_hunk_new_start(raw_line) {
        state.added_line_no = plus_line;
        return;
    }
    if let Some(added) = raw_line.strip_prefix('+') {
        let location = format!("{}:{}", state.commit, state.current_file);
        let line_no = if state.added_line_no > 0 { state.added_line_no } else { 1 };
        scan_line(&location, line_no, added, hits);
        state.added_line_no = state.added_line_no.saturating_add(1);
    }
}

fn parse_diff_plus_path(raw_line: &str) -> Option<String> {
    let rest = raw_line.strip_prefix("+++ ")?.trim_end();
    if rest == "/dev/null" {
        return Some(String::from("<deleted>"));
    }
    if let Some(unquoted) = rest.strip_prefix("b/") {
        return Some(unquoted.to_owned());
    }
    if rest.starts_with('"') && rest.ends_with('"') && rest.len() >= 2 {
        let decoded = unescape_git_quoted_path(&rest[1..rest.len() - 1]);
        return Some(decoded.strip_prefix("b/").unwrap_or(&decoded).to_owned());
    }
    Some(rest.to_owned())
}

fn unescape_git_quoted_path(escaped: &str) -> String {
    let bytes = escaped.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            match bytes[i + 1] {
                b'a' => {
                    out.push(0x07);
                    i += 2;
                }
                b'b' => {
                    out.push(0x08);
                    i += 2;
                }
                b't' => {
                    out.push(b'\t');
                    i += 2;
                }
                b'n' => {
                    out.push(b'\n');
                    i += 2;
                }
                b'v' => {
                    out.push(0x0b);
                    i += 2;
                }
                b'f' => {
                    out.push(0x0c);
                    i += 2;
                }
                b'r' => {
                    out.push(b'\r');
                    i += 2;
                }
                b'"' => {
                    out.push(b'"');
                    i += 2;
                }
                b'\\' => {
                    out.push(b'\\');
                    i += 2;
                }
                b'0'..=b'7' if i + 3 < bytes.len() && bytes[i + 2].is_ascii_digit() && bytes[i + 3].is_ascii_digit() => {
                    if let Ok(oct) = std::str::from_utf8(&bytes[i + 1..i + 4])
                        && let Ok(val) = u8::from_str_radix(oct, 8)
                    {
                        out.push(val);
                        i += 4;
                        continue;
                    }
                    out.push(bytes[i]);
                    i += 1;
                }
                other => {
                    out.push(other);
                    i += 2;
                }
            }
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn parse_hunk_new_start(line: &str) -> Option<usize> {
    let rest = line.strip_prefix("@@ ")?;
    let plus_part = rest.split_whitespace().find(|tok| tok.starts_with('+'))?;
    let num = plus_part.trim_start_matches('+').split(',').next()?;
    num.parse().ok()
}

/// Scan a complete text buffer line-by-line for secrets and local home paths.
pub fn scan_text(location: &str, text: &str, out: &mut Vec<SecretHit>) {
    for (idx, line) in text.lines().enumerate() {
        scan_line(location, idx + 1, line, out);
    }
}

fn scan_line(location: &str, line_no: usize, line: &str, out: &mut Vec<SecretHit>) {
    if contains_pem_private_key(line) {
        out.push(SecretHit { location: location.to_owned(), line: line_no, kind: "PEM private key header" });
    }
    if let Some(kind) = detect_api_token(line) {
        out.push(SecretHit { location: location.to_owned(), line: line_no, kind });
    }
    if detect_inline_secret_assignment(line) {
        out.push(SecretHit { location: location.to_owned(), line: line_no, kind: "inline secret env assignment" });
    }
    if let Some(kind) = detect_home_path(line) {
        out.push(SecretHit { location: location.to_owned(), line: line_no, kind });
    }
}

fn contains_pem_private_key(line: &str) -> bool {
    let Some(start) = line.find(PEM_BEGIN_PREFIX) else {
        return false;
    };
    let after = &line[start + PEM_BEGIN_PREFIX.len()..];
    let Some(end) = after.find(PEM_PRIVATE_KEY_SUFFIX) else {
        return false;
    };
    let header_kind = &after[..end];
    header_kind.len() <= 40 && header_kind.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b' ' || b == b'-')
}

fn detect_api_token(line: &str) -> Option<&'static str> {
    if has_prefixed_token(line, "sk-", true, 4, is_token_body_dash) {
        return Some("API token (`sk-...`)");
    }
    for prefix in ["ghp_", "gho_", "ghu_", "ghs_", "ghr_"] {
        if has_prefixed_token(line, prefix, false, 6, is_token_body_und) {
            return Some("GitHub token (`gh*_...`)");
        }
    }
    if has_prefixed_token(line, "github_pat_", false, 6, is_token_body_und) {
        return Some("GitHub fine-grained token (`github_pat_...`)");
    }
    if has_aws_access_key(line) {
        return Some("AWS access key (`AKIA...`/`ASIA...`)");
    }
    if has_prefixed_token(line, "AIza", false, 8, is_token_body_dash) {
        return Some("Google API key (`AIza...`)");
    }
    if has_slack_token(line) {
        return Some("Slack token (`xox...`)");
    }
    None
}

fn has_prefixed_token(line: &str, prefix: &str, reject_hyphen_left: bool, min_tail: usize, body_pred: fn(u8) -> bool) -> bool {
    let bytes = line.as_bytes();
    let mut start = 0;
    while let Some(rel) = line[start..].find(prefix) {
        let pos = start + rel;
        let left_ok = if pos == 0 {
            true
        } else {
            let prev = bytes[pos - 1];
            !prev.is_ascii_alphanumeric() && prev != b'_' && (!reject_hyphen_left || prev != b'-')
        };
        if left_ok {
            let tail = &bytes[pos + prefix.len()..];
            let count = tail.iter().take_while(|&&b| body_pred(b)).count();
            if count >= min_tail {
                return true;
            }
        }
        start = pos + 1;
    }
    false
}

fn has_aws_access_key(line: &str) -> bool {
    let bytes = line.as_bytes();
    for prefix in ["AKIA", "ASIA"] {
        let mut start = 0;
        while let Some(rel) = line[start..].find(prefix) {
            let pos = start + rel;
            let left_ok = pos == 0 || (!bytes[pos - 1].is_ascii_alphanumeric() && bytes[pos - 1] != b'_');
            let tail_start = pos + 4;
            if left_ok && bytes.len() >= tail_start + 16 {
                let key_body = &bytes[tail_start..tail_start + 16];
                let right_ok = bytes.get(tail_start + 16).is_none_or(|&b| !b.is_ascii_ascii_upper_alnum());
                if right_ok && key_body.iter().all(u8::is_ascii_ascii_upper_alnum) {
                    return true;
                }
            }
            start = pos + 1;
        }
    }
    false
}

trait AsciiUpperAlnum {
    fn is_ascii_ascii_upper_alnum(&self) -> bool;
}

impl AsciiUpperAlnum for u8 {
    fn is_ascii_ascii_upper_alnum(&self) -> bool {
        self.is_ascii_uppercase() || self.is_ascii_digit()
    }
}

fn has_slack_token(line: &str) -> bool {
    let bytes = line.as_bytes();
    let mut start = 0;
    while let Some(rel) = line[start..].find("xox") {
        let pos = start + rel;
        let left_ok = pos == 0 || (!bytes[pos - 1].is_ascii_alphanumeric() && bytes[pos - 1] != b'_' && bytes[pos - 1] != b'-');
        if left_ok
            && let Some(&kind_ch) = bytes.get(pos + 3)
            && matches!(kind_ch, b'b' | b'a' | b'p' | b'r' | b's')
            && bytes.get(pos + 4) == Some(&b'-')
        {
            let tail = &bytes[pos + 5..];
            let count = tail.iter().take_while(|&&b| is_token_body_dash(b)).count();
            if count >= 6 {
                return true;
            }
        }
        start = pos + 1;
    }
    false
}

const fn is_token_body_dash(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

const fn is_token_body_und(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn detect_inline_secret_assignment(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with("//") {
        return false;
    }
    let (is_rust_decl, body) = strip_decl_prefixes(trimmed);
    let Some((raw_lhs, raw_rhs)) = split_assignment(body, is_rust_decl) else {
        return false;
    };
    let lhs_no_type = if is_rust_decl { raw_lhs.split(':').next().unwrap_or(raw_lhs) } else { raw_lhs };
    let lhs = lhs_no_type.trim().trim_matches(|c| c == '"' || c == '\'');
    if !is_secret_env_key(lhs) {
        return false;
    }
    if is_rust_decl && !rhs_starts_with_string_literal(raw_rhs.trim()) {
        return false;
    }
    let (rhs, was_quoted) = extract_assignment_value(raw_rhs);
    !is_placeholder_value(rhs, was_quoted)
}

fn strip_decl_prefixes(s: &str) -> (bool, &str) {
    let mut rest = s.trim_start();
    if let Some(after_pub) = strip_rust_pub_prefix(rest) {
        rest = after_pub.trim_start();
    }
    for kw in ["const ", "static mut ", "static ", "let mut ", "let "] {
        if let Some(after_kw) = rest.strip_prefix(kw) {
            return (true, after_kw.trim_start());
        }
    }
    if let Some(after_export) = rest.strip_prefix("export ") {
        return (false, after_export.trim_start());
    }
    (false, rest)
}

fn strip_rust_pub_prefix(s: &str) -> Option<&str> {
    if let Some(rest) = s.strip_prefix("pub ") {
        return Some(rest);
    }
    if let Some(after_pub) = s.strip_prefix("pub(")
        && let Some(close) = after_pub.find(')')
    {
        return Some(after_pub[close + 1..].trim_start());
    }
    None
}

fn rhs_starts_with_string_literal(trimmed_rhs: &str) -> bool {
    trimmed_rhs.starts_with('"') || trimmed_rhs.starts_with("r\"") || trimmed_rhs.starts_with("r#\"") || trimmed_rhs.starts_with('\'')
}

fn split_assignment(s: &str, eq_only: bool) -> Option<(&str, &str)> {
    let eq_pos = s.find('=');
    if eq_only {
        return eq_pos.map(|idx| (&s[..idx], &s[idx + 1..]));
    }
    let colon_pos = s.find(':');
    let idx = match (eq_pos, colon_pos) {
        (Some(e), Some(c)) => e.min(c),
        (Some(e), None) => e,
        (None, Some(c)) => c,
        (None, None) => return None,
    };
    Some((&s[..idx], &s[idx + 1..]))
}

fn is_secret_env_key(key: &str) -> bool {
    if key.len() < 5 || !key.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_') {
        return false;
    }
    EXACT_SECRET_KEYS.contains(&key) || SECRET_KEY_SUFFIXES.iter().any(|suffix| key.ends_with(suffix))
}

fn extract_assignment_value(raw_rhs: &str) -> (&str, bool) {
    let trimmed = raw_rhs.trim();
    if let Some(after_raw) = trimmed.strip_prefix("r#\"") {
        return (after_raw.split("\"#").next().unwrap_or(""), true);
    }
    if let Some(after_raw) = trimmed.strip_prefix("r\"") {
        return (after_raw.split('"').next().unwrap_or(""), true);
    }
    if let Some(after_dq) = trimmed.strip_prefix('"') {
        return (after_dq.split('"').next().unwrap_or(""), true);
    }
    if let Some(after_sq) = trimmed.strip_prefix('\'') {
        return (after_sq.split('\'').next().unwrap_or(""), true);
    }
    let before_comment = trimmed.split('#').next().unwrap_or("").trim();
    (before_comment.trim_end_matches([',', ';']).trim(), false)
}

fn is_placeholder_value(val: &str, was_quoted: bool) -> bool {
    let trimmed = val.trim();
    if trimmed.len() < 6 {
        return true;
    }
    if !was_quoted && (trimmed.contains(char::is_whitespace) || trimmed.contains('(') || trimmed.contains(')') || trimmed.contains("::")) {
        return true;
    }
    if trimmed.starts_with('$') || trimmed.starts_with('<') || trimmed.starts_with('{') || trimmed.starts_with("secrets.") {
        return true;
    }
    let lower = trimmed.to_ascii_lowercase();
    if EXACT_PLACEHOLDER_VALUES.contains(&lower.as_str()) {
        return true;
    }
    if trimmed.bytes().all(|b| b == b'*' || b == b'x' || b == b'X' || b == b'.') {
        return true;
    }
    let parts: Vec<&str> = lower.split(|c: char| !c.is_ascii_alphanumeric()).filter(|s| !s.is_empty()).collect();
    !parts.is_empty()
        && (parts.iter().any(|p| matches!(*p, "example" | "placeholder" | "changeme" | "dummy" | "sample" | "redacted"))
            || lower.starts_with("your_")
            || lower.starts_with("your-"))
        && parts.iter().all(|p| PLACEHOLDER_WORDS.contains(p))
}

fn detect_home_path(line: &str) -> Option<&'static str> {
    if has_unix_home_prefix(line, "/Users/") {
        return Some("local macOS home path (`/Users/<name>/`)");
    }
    if has_unix_home_prefix(line, "/home/") {
        return Some("local Linux home path (`/home/<name>/`)");
    }
    if has_windows_home_prefix(line) {
        return Some("local Windows home path (`C:\\Users\\<name>\\`)");
    }
    None
}

fn has_unix_home_prefix(line: &str, prefix: &str) -> bool {
    let bytes = line.as_bytes();
    let mut start = 0;
    while let Some(rel) = line[start..].find(prefix) {
        let pos = start + rel;
        let left_ok = pos == 0 || !bytes[pos - 1].is_ascii_alphanumeric();
        if left_ok {
            let tail = &bytes[pos + prefix.len()..];
            let name_len = tail.iter().take_while(|&&b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-')).count();
            if name_len > 0 && tail.get(name_len) == Some(&b'/') {
                return true;
            }
        }
        start = pos + 1;
    }
    false
}

fn has_windows_home_prefix(line: &str) -> bool {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i + 9 < bytes.len() {
        let drive_ok = bytes[i].is_ascii_alphabetic() && bytes[i + 1] == b':';
        let left_ok = i == 0 || !bytes[i - 1].is_ascii_alphanumeric();
        if drive_ok && left_ok {
            let sep = bytes[i + 2];
            if (sep == b'\\' || sep == b'/')
                && bytes.get(i + 3..i + 8).is_some_and(|w| w.eq_ignore_ascii_case(b"users"))
                && bytes.get(i + 8) == Some(&sep)
            {
                let tail = &bytes[i + 9..];
                let name_len = tail.iter().take_while(|&&b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-')).count();
                if name_len > 0 && tail.get(name_len) == Some(&sep) {
                    return true;
                }
            }
        }
        i += 1;
    }
    false
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{scan_git_diff, scan_text};

    #[test]
    fn catches_fake_tokens_pem_env_secrets_and_home_paths() {
        let fake_openai = concat!("sk-", "test", "1234567890");
        let fake_ghp = concat!("ghp_", "AbCdEfGh123456");
        let fake_ghpat = concat!("github_pat_", "11AABBCCDD");
        let fake_aws = concat!("AKIA", "IOSFODNN7EXAMPLE");
        let fake_google = concat!("AIza", "SyFakeKey12345");
        let fake_slack = concat!("xoxb-", "123456-abcdef");
        let fake_pem = concat!("-----", "BEGIN RSA ", "PRIVATE ", "KEY-----");
        let fake_env = concat!("OPENAI_API", "_KEY=", "super_secret_token_999");
        let fake_mac = concat!("/Users/", "alice/work/file.txt");
        let fake_linux = concat!("/home/", "bob/.ssh/id");
        let fake_win = concat!("C:\\Users\\", "carol\\Documents");

        for sample in [fake_openai, fake_ghp, fake_ghpat, fake_aws, fake_google, fake_slack, fake_pem, fake_env, fake_mac, fake_linux, fake_win] {
            let mut hits = Vec::new();
            scan_text("fixture.txt", sample, &mut hits);
            assert!(!hits.is_empty(), "expected secret detector to catch `{sample}`");
        }
    }

    #[test]
    fn ignores_legitimate_prose_placeholders_and_own_source() {
        let safe = "\
disk-full and risk-difference and asterisk-excluded
GITHUB_TOKEN=${{ github.token }}
APPLE_CERTIFICATE_PASSWORD=${APPLE_CERTIFICATE_PASSWORD}
API_KEY=changeme
API_KEY=<your-api-key>
path template /Users/<name>/workspace and C:\\Users\\<name>\\
let token = raw.trim();
";
        let mut hits = Vec::new();
        scan_text("safe.txt", safe, &mut hits);
        assert!(hits.is_empty(), "unexpected false positives: {hits:?}");

        let own_src = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/secrets.rs")).unwrap();
        let mut own_hits = Vec::new();
        scan_text("xtask/src/secrets.rs", &own_src, &mut own_hits);
        assert!(own_hits.is_empty(), "secrets.rs must not false-positive on itself: {own_hits:?}");
    }

    #[test]
    fn scans_git_diff_additions_only() {
        let fake_sk = concat!("sk-", "test", "abcdef12");
        let fake_path = concat!("/Users/", "alice/project");
        let diff = format!(
            "commit abcdef1234567890\n\
             +++ b/src/cfg.rs\n\
             @@ -1,0 +1,2 @@\n\
             -let old = \"{fake_sk}\";\n\
             +let key = \"{fake_sk}\";\n\
             +let p = \"{fake_path}\";\n"
        );
        let hits = scan_git_diff(&diff);
        assert_eq!(hits.len(), 2, "{hits:?}");
        assert_eq!(hits[0].location, "abcdef123456:src/cfg.rs");
        assert_eq!(hits[0].line, 1);
        assert_eq!(hits[1].line, 2);
    }

    #[test]
    fn r2_s13_10_rust_const_let_unmasked_substrings_and_quoted_diff_paths() {
        let rust_const = format!("pub const AZURE_CLIENT_{}: &str = \"{}\";", "SECRET", concat!("8Q~real_production_", "value_9182"));
        let rust_let = format!("let mut API_{}: &str = \"{}\";", "SECRET", concat!("secret_prod_", "key_4412"));
        let parens_pw = format!("WINDOWS_CERTIFICATE_{}=\"{}\"", "PASSWORD", concat!("p@ss(word)", "Prod2026!"));
        let none_sub = format!("API_{}=\"{}\"", "SECRET", concat!("k9_None_", "ProdKey_88412"));
        let gho_tok = concat!("gho_", "AbCdEf123456");
        let asia_tok = concat!("ASIA", "IOSFODNN7EXAMPLE");

        for line in [&rust_const, &rust_let, &parens_pw, &none_sub, gho_tok, asia_tok] {
            let mut hits = Vec::new();
            scan_text("test.rs", line, &mut hits);
            assert!(!hits.is_empty(), "expected finding for `{line}`");
        }

        let safe = format!("let {} = raw.trim();\nlet API_KEY = env::var(\"X\");\nAPI_{}=\"your_api_key_here\"\n", "token", "KEY");
        let mut safe_hits = Vec::new();
        scan_text("safe.rs", &safe, &mut safe_hits);
        assert!(safe_hits.is_empty(), "unexpected hits: {safe_hits:?}");

        let fake_sk = concat!("sk-", "test", "abcdef12");
        let quoted_diff = format!("commit 1234567890ab\n+++ \"b/src/caf\\303\\251/cfg.rs\"\n@@ -1,0 +1,1 @@\n+let key = \"{fake_sk}\";\n");
        let qhits = scan_git_diff(&quoted_diff);
        assert_eq!(qhits.len(), 1);
        assert_eq!(qhits[0].location, "1234567890ab:src/café/cfg.rs");
    }
}
