//! Just enough JSON for the language server protocol.
//!
//! The server has no dependencies on purpose (the workspace builds offline),
//! and LSP only needs a small subset of JSON: objects, arrays, strings,
//! numbers, booleans, and null. This module parses that subset and writes it
//! back out. It is deliberately not a general-purpose library: there are no
//! streaming APIs, no `serde` attributes, and numbers are `f64`.

/// A JSON value.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Value>),
    Obj(Vec<(String, Value)>),
}

impl Value {
    /// The value under `key`, when this is an object.
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Obj(fields) => fields
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value),
            _ => None,
        }
    }

    /// Follow a path of object keys.
    pub fn get_path(&self, keys: &[&str]) -> Option<&Value> {
        let mut current = self;
        for key in keys {
            current = current.get(key)?;
        }
        Some(current)
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(text) => Some(text),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Num(number) => Some(*number as i64),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Arr(items) => Some(items),
            _ => None,
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }
}

/// Build an object from pairs.
pub fn object<const N: usize>(fields: [(&str, Value); N]) -> Value {
    Value::Obj(
        fields
            .into_iter()
            .map(|(name, value)| (name.to_string(), value))
            .collect(),
    )
}

/// Build a string value.
pub fn string(text: impl Into<String>) -> Value {
    Value::Str(text.into())
}

/// Build a number value.
pub fn number(value: i64) -> Value {
    Value::Num(value as f64)
}

/// Parse one JSON document.
pub fn parse(text: &str) -> Result<Value, String> {
    let mut parser = Parser {
        bytes: text.as_bytes(),
        at: 0,
    };
    let value = parser.value()?;
    parser.whitespace();
    if parser.at != parser.bytes.len() {
        return Err(format!(
            "trailing bytes at offset {} after the JSON value",
            parser.at
        ));
    }
    Ok(value)
}

struct Parser<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Parser<'_> {
    fn whitespace(&mut self) {
        while matches!(self.bytes.get(self.at), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.at += 1;
        }
    }

    fn value(&mut self) -> Result<Value, String> {
        self.whitespace();
        match self.bytes.get(self.at) {
            Some(b'{') => self.object(),
            Some(b'[') => self.array(),
            Some(b'"') => Ok(Value::Str(self.string()?)),
            Some(b't') => self.literal("true", Value::Bool(true)),
            Some(b'f') => self.literal("false", Value::Bool(false)),
            Some(b'n') => self.literal("null", Value::Null),
            Some(byte) if *byte == b'-' || byte.is_ascii_digit() => self.number(),
            Some(byte) => Err(format!(
                "expected a JSON value at offset {}, found `{}`",
                self.at, *byte as char
            )),
            None => Err("the JSON document ended where a value was expected".to_string()),
        }
    }

    fn literal(&mut self, text: &str, value: Value) -> Result<Value, String> {
        if self.bytes[self.at..].starts_with(text.as_bytes()) {
            self.at += text.len();
            Ok(value)
        } else {
            Err(format!("expected `{text}` at offset {}", self.at))
        }
    }

    fn object(&mut self) -> Result<Value, String> {
        self.at += 1;
        let mut fields = Vec::new();
        self.whitespace();
        if self.bytes.get(self.at) == Some(&b'}') {
            self.at += 1;
            return Ok(Value::Obj(fields));
        }
        loop {
            self.whitespace();
            let key = self.string()?;
            self.whitespace();
            self.expect(b':')?;
            let value = self.value()?;
            fields.push((key, value));
            self.whitespace();
            match self.bytes.get(self.at) {
                Some(b',') => self.at += 1,
                Some(b'}') => {
                    self.at += 1;
                    return Ok(Value::Obj(fields));
                }
                _ => return Err(format!("expected `,` or `}}` at offset {}", self.at)),
            }
        }
    }

    fn array(&mut self) -> Result<Value, String> {
        self.at += 1;
        let mut items = Vec::new();
        self.whitespace();
        if self.bytes.get(self.at) == Some(&b']') {
            self.at += 1;
            return Ok(Value::Arr(items));
        }
        loop {
            items.push(self.value()?);
            self.whitespace();
            match self.bytes.get(self.at) {
                Some(b',') => self.at += 1,
                Some(b']') => {
                    self.at += 1;
                    return Ok(Value::Arr(items));
                }
                _ => return Err(format!("expected `,` or `]` at offset {}", self.at)),
            }
        }
    }

    fn expect(&mut self, byte: u8) -> Result<(), String> {
        if self.bytes.get(self.at) == Some(&byte) {
            self.at += 1;
            Ok(())
        } else {
            Err(format!(
                "expected `{}` at offset {}",
                byte as char, self.at
            ))
        }
    }

    fn string(&mut self) -> Result<String, String> {
        self.expect(b'"')?;
        let mut out = String::new();
        loop {
            let Some(byte) = self.bytes.get(self.at).copied() else {
                return Err("the JSON string is unterminated".to_string());
            };
            match byte {
                b'"' => {
                    self.at += 1;
                    return Ok(out);
                }
                b'\\' => {
                    self.at += 1;
                    let Some(escape) = self.bytes.get(self.at).copied() else {
                        return Err("the JSON escape is unterminated".to_string());
                    };
                    self.at += 1;
                    match escape {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{0008}'),
                        b'f' => out.push('\u{000c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let code = self.hex4()?;
                            // A high surrogate is followed by a low one; both
                            // together are one `char`.
                            if (0xD800..0xDC00).contains(&code) {
                                if self.bytes.get(self.at) == Some(&b'\\')
                                    && self.bytes.get(self.at + 1) == Some(&b'u')
                                {
                                    self.at += 2;
                                    let low = self.hex4()?;
                                    if (0xDC00..0xE000).contains(&low) {
                                        let combined = 0x10000
                                            + ((code - 0xD800) << 10)
                                            + (low - 0xDC00);
                                        out.push(
                                            char::from_u32(combined)
                                                .ok_or("invalid surrogate pair")?,
                                        );
                                    } else {
                                        return Err("invalid low surrogate".to_string());
                                    }
                                } else {
                                    return Err("unpaired high surrogate".to_string());
                                }
                            } else {
                                out.push(
                                    char::from_u32(code).ok_or("invalid unicode escape")?,
                                );
                            }
                        }
                        other => {
                            return Err(format!(
                                "unknown JSON escape `\\{}`",
                                other as char
                            ))
                        }
                    }
                }
                other => {
                    // Copy the whole UTF-8 sequence: a byte at a time would
                    // split a multibyte character.
                    let start = self.at;
                    let width = utf8_width(other);
                    let end = start + width;
                    let Some(text) = self
                        .bytes
                        .get(start..end)
                        .and_then(|slice| std::str::from_utf8(slice).ok())
                    else {
                        return Err(format!("invalid UTF-8 at offset {start}"));
                    };
                    out.push_str(text);
                    self.at = end;
                }
            }
        }
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let Some(digits) = self.bytes.get(self.at..self.at + 4) else {
            return Err("the unicode escape is truncated".to_string());
        };
        let text = std::str::from_utf8(digits).map_err(|_| "invalid unicode escape")?;
        let code = u32::from_str_radix(text, 16).map_err(|_| "invalid unicode escape")?;
        self.at += 4;
        Ok(code)
    }

    fn number(&mut self) -> Result<Value, String> {
        let start = self.at;
        if self.bytes.get(self.at) == Some(&b'-') {
            self.at += 1;
        }
        while matches!(self.bytes.get(self.at), Some(byte) if byte.is_ascii_digit()) {
            self.at += 1;
        }
        if self.bytes.get(self.at) == Some(&b'.') {
            self.at += 1;
            while matches!(self.bytes.get(self.at), Some(byte) if byte.is_ascii_digit()) {
                self.at += 1;
            }
        }
        if matches!(self.bytes.get(self.at), Some(b'e' | b'E')) {
            self.at += 1;
            if matches!(self.bytes.get(self.at), Some(b'+' | b'-')) {
                self.at += 1;
            }
            while matches!(self.bytes.get(self.at), Some(byte) if byte.is_ascii_digit()) {
                self.at += 1;
            }
        }
        let text = std::str::from_utf8(&self.bytes[start..self.at])
            .map_err(|_| "invalid number")?;
        text.parse::<f64>()
            .map(Value::Num)
            .map_err(|_| format!("`{text}` is not a JSON number"))
    }
}

fn utf8_width(first: u8) -> usize {
    if first < 0x80 {
        1
    } else if first >> 5 == 0b110 {
        2
    } else if first >> 4 == 0b1110 {
        3
    } else {
        4
    }
}

/// Write one JSON document, compactly.
pub fn to_string(value: &Value) -> String {
    let mut out = String::new();
    write_value(value, &mut out);
    out
}

fn write_value(value: &Value, out: &mut String) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Num(number) => {
            if number.fract() == 0.0 && number.abs() < 9.0e15 {
                out.push_str(&format!("{}", *number as i64));
            } else {
                out.push_str(&format!("{number}"));
            }
        }
        Value::Str(text) => write_string(text, out),
        Value::Arr(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_value(item, out);
            }
            out.push(']');
        }
        Value::Obj(fields) => {
            out.push('{');
            for (index, (name, item)) in fields.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_string(name, out);
                out.push(':');
                write_value(item, out);
            }
            out.push('}');
        }
    }
}

fn write_string(text: &str, out: &mut String) {
    out.push('"');
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other if (other as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", other as u32));
            }
            other => out.push(other),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_round_trip_keeps_the_value() {
        let text = r#"{"id":1,"method":"hover","params":{"position":{"line":3,"character":14},"text":"a\nb \"q\" \\ ü"}}"#;
        let value = parse(text).expect("the document must parse");
        assert_eq!(to_string(&value).as_str(), text);
    }

    #[test]
    fn a_surrogate_pair_becomes_one_char() {
        let value = parse(r#""\ud83d\ude00""#).expect("the document must parse");
        assert_eq!(value, Value::Str("😀".to_string()));
    }

    #[test]
    fn a_trailing_comma_is_an_error() {
        assert!(parse(r#"{"a":1,}"#).is_err());
    }
}
