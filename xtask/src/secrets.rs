//! `cargo xtask secrets [--history]`: scan tracked files (or git history additions) for API
//! tokens, PEM private keys, inline secret env assignments, and local home-folder paths.

use std::path::Path;
use std::process::Command;

const PEM_BEGIN_PREFIX: &str = concat!("-----", "BEGIN ");
const PEM_PRIVATE_KEY_SUFFIX: &str = concat!("PRIVATE ", "KEY-----");

const SECRET_KEY_SUFFIXES: &[&str] =
    &["_API_KEY", "_SECRET_KEY", "_ACCESS_KEY", "_PRIVATE_KEY", "_AUTH_TOKEN", "_SECRET", "_TOKEN", "_PASSWORD", "_PASSWD"];
const EXACT_SECRET_KEYS: &[&str] = &["API_KEY", "SECRET_KEY", "PRIVATE_KEY", "AUTH_TOKEN", "ACCESS_TOKEN", "PASSWORD", "SECRET"];

const PLACEHOLDER_TOKENS: &[&str] = &[
    "example",
    "placeholder",
    "changeme",
    "replace_me",
    "replace-me",
    "your_",
    "your-",
    "redacted",
    "dummy",
    "sample",
    "not-set",
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
    let out =
        Command::new("git").current_dir(root).args(["log", "-p", "--unified=0"]).output().map_err(|e| format!("git log -p --unified=0: {e}"))?;
    if !out.status.success() {
        return Err(format!("git log -p --unified=0 failed: {}", String::from_utf8_lossy(&out.stderr)));
    }
    let diff = String::from_utf8_lossy(&out.stdout);
    let hits = scan_git_diff(&diff);
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

/// Scan added (`+`) lines in `git log -p --unified=0` output.
#[must_use]
pub fn scan_git_diff(diff: &str) -> Vec<SecretHit> {
    let mut hits = Vec::new();
    let mut commit = String::from("HEAD");
    let mut current_file = String::from("<unknown>");
    let mut added_line_no = 0usize;

    for raw_line in diff.lines() {
        if let Some(rest) = raw_line.strip_prefix("commit ") {
            let short = rest.split_whitespace().next().unwrap_or(rest);
            commit = short.chars().take(12).collect();
            continue;
        }
        if let Some(path) = raw_line.strip_prefix("+++ b/") {
            current_file = path.to_owned();
            continue;
        }
        if raw_line.starts_with("+++ ") || raw_line.starts_with("--- ") {
            continue;
        }
        if let Some(plus_line) = parse_hunk_new_start(raw_line) {
            added_line_no = plus_line;
            continue;
        }
        if let Some(added) = raw_line.strip_prefix('+') {
            let location = format!("{commit}:{current_file}");
            let line_no = if added_line_no > 0 { added_line_no } else { 1 };
            scan_line(&location, line_no, added, &mut hits);
            added_line_no = added_line_no.saturating_add(1);
        }
    }
    hits
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
    if has_prefixed_token(line, "ghp_", false, 6, is_token_body_und) {
        return Some("GitHub token (`ghp_...`)");
    }
    if has_prefixed_token(line, "github_pat_", false, 6, is_token_body_und) {
        return Some("GitHub fine-grained token (`github_pat_...`)");
    }
    if has_aws_access_key(line) {
        return Some("AWS access key (`AKIA...`)");
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
    let mut start = 0;
    while let Some(rel) = line[start..].find("AKIA") {
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
    if trimmed.is_empty()
        || trimmed.starts_with('#')
        || trimmed.starts_with("//")
        || trimmed.starts_with("const ")
        || trimmed.starts_with("pub const ")
        || trimmed.starts_with("static ")
        || trimmed.starts_with("pub static ")
        || trimmed.starts_with("let ")
    {
        return false;
    }
    let body = trimmed.strip_prefix("export ").map(str::trim_start).unwrap_or(trimmed);
    let Some((raw_lhs, raw_rhs)) = split_assignment(body) else {
        return false;
    };
    let lhs = raw_lhs.trim().trim_matches(|c| c == '"' || c == '\'');
    if !is_secret_env_key(lhs) {
        return false;
    }
    let rhs = extract_assignment_value(raw_rhs);
    !is_placeholder_value(rhs)
}

fn split_assignment(s: &str) -> Option<(&str, &str)> {
    let eq_pos = s.find('=');
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

fn extract_assignment_value(raw_rhs: &str) -> &str {
    let trimmed = raw_rhs.trim();
    if let Some(after_dq) = trimmed.strip_prefix('"') {
        return after_dq.split('"').next().unwrap_or("");
    }
    if let Some(after_sq) = trimmed.strip_prefix('\'') {
        return after_sq.split('\'').next().unwrap_or("");
    }
    let before_comment = trimmed.split('#').next().unwrap_or("").trim();
    before_comment.trim_end_matches([',', ';']).trim()
}

fn is_placeholder_value(val: &str) -> bool {
    if val.len() < 6 || val.contains(char::is_whitespace) {
        return true;
    }
    if val.starts_with('$') || val.starts_with('<') || val.starts_with('{') {
        return true;
    }
    if val.contains('(') || val.contains(')') || val.contains("::") || val.contains("secrets.") {
        return true;
    }
    let lower = val.to_ascii_lowercase();
    if PLACEHOLDER_TOKENS.iter().any(|p| lower.contains(p)) {
        return true;
    }
    val.bytes().all(|b| b == b'*' || b == b'x' || b == b'X' || b == b'.')
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
}
