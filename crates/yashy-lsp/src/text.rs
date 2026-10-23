//! Byte offsets, LSP positions, and document URIs.
//!
//! LSP positions are `(line, character)` pairs where `character` counts UTF-16
//! code units, and the compiler reports byte offsets. The conversion lives here
//! so the server never guesses: a line index gives the byte offset each line
//! starts at, and a position walks that line one character at a time.

use std::path::{Path, PathBuf};

/// The byte offset each line starts at. The last entry is the end of the text.
pub struct LineIndex {
    starts: Vec<usize>,
}

impl LineIndex {
    pub fn new(text: &str) -> Self {
        let mut starts = vec![0];
        for (offset, byte) in text.bytes().enumerate() {
            if byte == b'\n' {
                starts.push(offset + 1);
            }
        }
        Self { starts }
    }

    /// The zero-based line and the UTF-16 column of `offset`.
    pub fn position(&self, text: &str, offset: usize) -> (u32, u32) {
        let offset = offset.min(text.len());
        let line = match self.starts.binary_search(&offset) {
            Ok(index) => index,
            Err(index) => index.saturating_sub(1),
        };
        let start = self.starts.get(line).copied().unwrap_or(0);
        let column = text
            .get(start..offset)
            .map(|slice| slice.encode_utf16().count())
            .unwrap_or(0);
        (line as u32, column as u32)
    }

    /// The byte offset of a zero-based line and UTF-16 column, if they exist.
    pub fn offset(&self, text: &str, line: u32, character: u32) -> Option<usize> {
        let start = *self.starts.get(line as usize)?;
        let end = self
            .starts
            .get(line as usize + 1)
            .copied()
            .unwrap_or(text.len());
        let line_text = text.get(start..end)?;
        let mut units = 0u32;
        for (index, character_in_line) in line_text.char_indices() {
            if units >= character {
                return Some(start + index);
            }
            units += character_in_line.len_utf16() as u32;
        }
        if units >= character {
            Some(end)
        } else {
            None
        }
    }
}

/// The identifier that contains `offset`, if any.
pub fn word_at(text: &str, offset: usize) -> Option<&str> {
    let bytes = text.as_bytes();
    let mut start = offset.min(bytes.len());
    while start > 0 && is_word_byte(bytes[start - 1]) {
        start -= 1;
    }
    let mut end = offset.min(bytes.len());
    while end < bytes.len() && is_word_byte(bytes[end]) {
        end += 1;
    }
    if start == end {
        return None;
    }
    text.get(start..end)
}

fn is_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// `file:///D:/Yashy/main.ya` for a local path.
///
/// Percent-encoding covers the characters a URI cannot carry unescaped; spaces
/// are the common case on Windows. A path that is not valid UTF-8 is lossily
/// converted, which is the best a URI can do.
pub fn path_to_uri(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    let mut uri = String::from("file://");
    if !text.starts_with('/') {
        uri.push('/');
    }
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/'
            | b':' => uri.push(byte as char),
            other => uri.push_str(&format!("%{other:02X}")),
        }
    }
    uri
}

/// The local path a `file:` URI names.
pub fn uri_to_path(uri: &str) -> PathBuf {
    let rest = uri.strip_prefix("file://").unwrap_or(uri);
    let mut decoded = String::new();
    let bytes = rest.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if let Some(hex) = bytes.get(index + 1..index + 3) {
                if let Ok(text) = std::str::from_utf8(hex) {
                    if let Ok(value) = u8::from_str_radix(text, 16) {
                        decoded.push(value as char);
                        index += 3;
                        continue;
                    }
                }
            }
        }
        decoded.push(bytes[index] as char);
        index += 1;
    }
    // `/D:/dir/file` is a Windows path with a leading slash the URI needs.
    if decoded.len() >= 3 {
        let bytes = decoded.as_bytes();
        if bytes[0] == b'/' && bytes[1].is_ascii_alphabetic() && bytes[2] == b':' {
            decoded.remove(0);
        }
    }
    PathBuf::from(decoded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_position_counts_utf16_units() {
        let text = "a😀b\nsecond";
        let index = LineIndex::new(text);
        let offset = text.find('b').expect("b is in the text");
        assert_eq!(index.position(text, offset), (0, 3));
        assert_eq!(index.offset(text, 0, 3), Some(offset));
        assert_eq!(index.position(text, text.find('s').unwrap()), (1, 0));
    }

    #[test]
    fn a_windows_path_becomes_a_uri() {
        let path = PathBuf::from(r"D:\Yashy dir\main.ya");
        let uri = path_to_uri(&path);
        assert_eq!(uri, "file:///D:/Yashy%20dir/main.ya");
        // Decoding gives back the same characters. Which separator the host's
        // `Path` spells them with is the host's business, not the URI's: on
        // Unix a Windows path is not a path at all, so only the characters can
        // be compared.
        assert_eq!(
            uri_to_path(&uri).to_string_lossy().replace('\\', "/"),
            "D:/Yashy dir/main.ya"
        );
    }

    #[test]
    fn a_native_path_round_trips() {
        let path = std::env::temp_dir().join("yashy lsp").join("main.ya");
        let uri = path_to_uri(&path);
        assert_eq!(uri_to_path(&uri), path);
    }
}
