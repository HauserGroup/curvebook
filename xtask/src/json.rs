//! Minimal, panic-free JSON parser (`std` only) for reading `cargo metadata --no-deps`.

use std::collections::BTreeMap;

/// Parsed JSON value.
#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Number(String),
    Str(String),
    Array(Vec<Json>),
    Object(BTreeMap<String, Json>),
}

impl Json {
    /// Look up an object field by key.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Object(map) => map.get(key),
            _ => None,
        }
    }

    /// Borrow as string slice if this is `Json::Str`.
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s.as_str()),
            _ => None,
        }
    }

    /// Borrow as slice if this is `Json::Array`.
    #[must_use]
    pub fn as_array(&self) -> Option<&[Json]> {
        match self {
            Json::Array(items) => Some(items.as_slice()),
            _ => None,
        }
    }

    /// Whether this value is `Json::Str`.
    #[must_use]
    pub const fn is_string(&self) -> bool {
        matches!(self, Json::Str(_))
    }
}

/// Parse a complete UTF-8 JSON document with bounded nesting depth.
pub fn parse(input: &str) -> Result<Json, String> {
    let mut parser = Parser { bytes: input.as_bytes(), pos: 0 };
    let val = parser.parse_value(0)?;
    parser.skip_ws();
    if parser.pos != parser.bytes.len() {
        return Err(format!("trailing bytes at offset {}", parser.pos));
    }
    Ok(val)
}

struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

const MAX_DEPTH: usize = 128;

impl Parser<'_> {
    fn skip_ws(&mut self) {
        while let Some(&b) = self.bytes.get(self.pos) {
            if matches!(b, b' ' | b'\t' | b'\r' | b'\n') {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn parse_value(&mut self, depth: usize) -> Result<Json, String> {
        if depth > MAX_DEPTH {
            return Err(String::from("JSON nesting depth exceeded"));
        }
        self.skip_ws();
        match self.peek() {
            Some(b'{') => self.parse_object(depth + 1),
            Some(b'[') => self.parse_array(depth + 1),
            Some(b'"') => self.parse_string().map(Json::Str),
            Some(b't' | b'f') => self.parse_bool(),
            Some(b'n') => self.parse_null(),
            Some(b'-' | b'0'..=b'9') => self.parse_number(),
            Some(other) => Err(format!("unexpected byte 0x{other:02x} at offset {}", self.pos)),
            None => Err(String::from("unexpected end of JSON input")),
        }
    }

    fn parse_object(&mut self, depth: usize) -> Result<Json, String> {
        self.pos += 1; // '{'
        self.skip_ws();
        let mut map = BTreeMap::new();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(Json::Object(map));
        }
        loop {
            self.skip_ws();
            if self.peek() != Some(b'"') {
                return Err(format!("expected object key string at offset {}", self.pos));
            }
            let key = self.parse_string()?;
            self.skip_ws();
            if self.peek() != Some(b':') {
                return Err(format!("expected ':' after key at offset {}", self.pos));
            }
            self.pos += 1;
            let val = self.parse_value(depth)?;
            map.insert(key, val);
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(Json::Object(map));
                }
                _ => return Err(format!("expected ',' or '}}' in object at offset {}", self.pos)),
            }
        }
    }

    fn parse_array(&mut self, depth: usize) -> Result<Json, String> {
        self.pos += 1; // '['
        self.skip_ws();
        let mut items = Vec::new();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(Json::Array(items));
        }
        loop {
            items.push(self.parse_value(depth)?);
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    return Ok(Json::Array(items));
                }
                _ => return Err(format!("expected ',' or ']' in array at offset {}", self.pos)),
            }
        }
    }

    fn parse_string(&mut self) -> Result<String, String> {
        self.pos += 1; // '"'
        let mut out = String::new();
        while let Some(&b) = self.bytes.get(self.pos) {
            self.pos += 1;
            match b {
                b'"' => return Ok(out),
                b'\\' => {
                    let esc = self.read_escape()?;
                    out.push(esc);
                }
                0x00..=0x1F => return Err(format!("unescaped control byte 0x{b:02x}")),
                ascii if ascii < 0x80 => out.push(char::from(ascii)),
                _ => {
                    let start = self.pos - 1;
                    let rest = std::str::from_utf8(self.bytes.get(start..).unwrap_or_default()).map_err(|e| format!("invalid UTF-8: {e}"))?;
                    let ch = rest.chars().next().ok_or_else(|| String::from("empty UTF-8 slice"))?;
                    self.pos = start + ch.len_utf8();
                    out.push(ch);
                }
            }
        }
        Err(String::from("unterminated JSON string"))
    }

    fn read_escape(&mut self) -> Result<char, String> {
        let Some(&e) = self.bytes.get(self.pos) else {
            return Err(String::from("unterminated string escape"));
        };
        self.pos += 1;
        match e {
            b'"' => Ok('"'),
            b'\\' => Ok('\\'),
            b'/' => Ok('/'),
            b'b' => Ok('\u{0008}'),
            b'f' => Ok('\u{000C}'),
            b'n' => Ok('\n'),
            b'r' => Ok('\r'),
            b't' => Ok('\t'),
            b'u' => self.read_unicode_escape(),
            other => Err(format!("invalid escape \\{}", char::from(other))),
        }
    }

    fn read_unicode_escape(&mut self) -> Result<char, String> {
        let hex = self.read_hex4()?;
        if (0xD800..=0xDBFF).contains(&hex) {
            if self.bytes.get(self.pos..self.pos + 2) != Some(b"\\u") {
                return Err(String::from("unpaired high surrogate in JSON string"));
            }
            self.pos += 2;
            let low = self.read_hex4()?;
            if !(0xDC00..=0xDFFF).contains(&low) {
                return Err(String::from("invalid low surrogate in JSON string"));
            }
            let cp = 0x10000 + (((u32::from(hex) - 0xD800) << 10) | (u32::from(low) - 0xDC00));
            return char::from_u32(cp).ok_or_else(|| String::from("invalid surrogate code point"));
        }
        char::from_u32(u32::from(hex)).ok_or_else(|| String::from("invalid unicode scalar"))
    }

    fn read_hex4(&mut self) -> Result<u16, String> {
        let slice = self.bytes.get(self.pos..self.pos + 4).ok_or_else(|| String::from("truncated \\uXXXX escape"))?;
        let text = std::str::from_utf8(slice).map_err(|_| String::from("non-ASCII hex escape"))?;
        let val = u16::from_str_radix(text, 16).map_err(|_| format!("invalid hex escape `{text}`"))?;
        self.pos += 4;
        Ok(val)
    }

    fn parse_bool(&mut self) -> Result<Json, String> {
        if self.bytes.get(self.pos..self.pos + 4) == Some(b"true") {
            self.pos += 4;
            Ok(Json::Bool(true))
        } else if self.bytes.get(self.pos..self.pos + 5) == Some(b"false") {
            self.pos += 5;
            Ok(Json::Bool(false))
        } else {
            Err(format!("invalid boolean token at offset {}", self.pos))
        }
    }

    fn parse_null(&mut self) -> Result<Json, String> {
        if self.bytes.get(self.pos..self.pos + 4) == Some(b"null") {
            self.pos += 4;
            Ok(Json::Null)
        } else {
            Err(format!("invalid null token at offset {}", self.pos))
        }
    }

    fn parse_number(&mut self) -> Result<Json, String> {
        let start = self.pos;
        while let Some(&b) = self.bytes.get(self.pos) {
            if matches!(b, b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E') {
                self.pos += 1;
            } else {
                break;
            }
        }
        let slice = self.bytes.get(start..self.pos).ok_or_else(|| String::from("invalid number span"))?;
        let text = std::str::from_utf8(slice).map_err(|_| String::from("invalid number UTF-8"))?;
        Ok(Json::Number(text.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::{Json, parse};

    #[test]
    fn parses_nested_metadata_shape() {
        let j = parse(r#"{"packages":[{"name":"curvebook-doc","kind":null,"path":"/x\\y"}]}"#).unwrap();
        let pkgs = j.get("packages").and_then(Json::as_array).unwrap();
        assert_eq!(pkgs.len(), 1);
        assert_eq!(pkgs[0].get("name").and_then(Json::as_str), Some("curvebook-doc"));
        assert_eq!(pkgs[0].get("path").and_then(Json::as_str), Some("/x\\y"));
        assert_eq!(pkgs[0].get("kind"), Some(&Json::Null));
    }
}
