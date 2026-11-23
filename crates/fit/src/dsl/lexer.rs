//! Tokenizer for DSL expressions.
//!
//! References:
//! - Aho, A. V., Lam, M. S., Sethi, R. & Ullman, J. D. (2006). *Compilers: Principles, Techniques, and
//!   Tools*, 2nd ed. Addison-Wesley.

use crate::dsl::ast::{Token, TokenKind};
use crate::error::FitError;

pub(in crate::dsl) fn tokenize(input: &str, line: usize) -> Result<Vec<Token>, FitError> {
    let bytes = input.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let ch = bytes[i];
        if ch.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let col = i + 1;
        if ch.is_ascii_digit() || (ch == b'.' && bytes.get(i + 1).is_some_and(u8::is_ascii_digit)) {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
                i += 1;
            }
            if i < bytes.len() && (bytes[i] == b'e' || bytes[i] == b'E') {
                let save = i;
                i += 1;
                if i < bytes.len() && (bytes[i] == b'+' || bytes[i] == b'-') {
                    i += 1;
                }
                if i < bytes.len() && bytes[i].is_ascii_digit() {
                    while i < bytes.len() && bytes[i].is_ascii_digit() {
                        i += 1;
                    }
                } else {
                    i = save;
                }
            }
            let slice = input.get(start..i).ok_or_else(|| FitError::DslCompileError {
                message: "Invalid numeric literal span".to_owned(),
                line,
                column: col,
            })?;
            let num = slice.parse::<f64>().map_err(|_| FitError::DslCompileError {
                message: format!("Invalid numeric literal `{slice}`"),
                line,
                column: col,
            })?;
            tokens.push(Token { kind: TokenKind::Number, text: slice.to_owned(), num, line, col });
            continue;
        }
        if ch.is_ascii_alphabetic() || ch == b'_' {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let ident = input.get(start..i).unwrap_or("");
            tokens.push(Token { kind: TokenKind::Ident, text: ident.to_owned(), num: 0.0, line, col });
            continue;
        }
        let next = bytes.get(i + 1).copied().unwrap_or(0);
        let (kind, len) = match (ch, next) {
            (b'=', b'=') => (TokenKind::EqEq, 2),
            (b'!', b'=') => (TokenKind::NotEq, 2),
            (b'<', b'>') => (TokenKind::NotEq, 2),
            (b'<', b'=') => (TokenKind::Le, 2),
            (b'>', b'=') => (TokenKind::Ge, 2),
            (b'&', b'&') => (TokenKind::AndAnd, 2),
            (b'|', b'|') => (TokenKind::OrOr, 2),
            (b'*', b'*') => (TokenKind::Caret, 2),
            (b'+', _) => (TokenKind::Plus, 1),
            (b'-', _) => (TokenKind::Minus, 1),
            (b'*', _) => (TokenKind::Star, 1),
            (b'/', _) => (TokenKind::Slash, 1),
            (b'%', _) => (TokenKind::Percent, 1),
            (b'^', _) => (TokenKind::Caret, 1),
            (b'(', _) => (TokenKind::LParen, 1),
            (b')', _) => (TokenKind::RParen, 1),
            (b',', _) => (TokenKind::Comma, 1),
            (b'=', _) => (TokenKind::Eq, 1),
            (b'<', _) => (TokenKind::Lt, 1),
            (b'>', _) => (TokenKind::Gt, 1),
            (b'!', _) => (TokenKind::Bang, 1),
            _ => {
                return Err(FitError::DslCompileError { message: format!("Unexpected character `{}`", ch as char), line, column: col });
            }
        };
        let text = input.get(i..i + len).unwrap_or("").to_owned();
        tokens.push(Token { kind, text, num: 0.0, line, col });
        i += len;
    }
    Ok(tokens)
}
