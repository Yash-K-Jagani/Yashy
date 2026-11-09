//! The TOML subset, parsed by a line-oriented state machine.
//!
//! The grammar is on one screen:
//!
//! ```text
//! file       := line*
//! line       := ws (comment | table | array-table | assignment) ws comment? ws
//! table      := '[' key ('.' key)* ']'
//! array-table:= '[[' key ('.' key)* ']]'
//! assignment := key ('.' key)* '=' ws value
//! key        := bare-key | '"' basic-string '"' | "'" literal-string "'"
//! value      := string | integer | boolean | array
//! array      := '[' ws (value ws (',' value ws)* ','?)? ']'
//! comment    := '#' any
//! ```
//!
//! Bare keys are `[A-Za-z0-9_-]+`. Strings are single-line; a basic string
//! understands `\\ \" \n \r \t`, and a literal string understands nothing.
//! Integers are decimal with an optional `-`; there are no floats and no
//! dates. Anything outside the subset is an error at its line and column.
//!
//! `[[package]]` is the one extension past the manifest grammar: an array of
//! tables is how a lockfile names a list of packages, and repeating the same
//! table header is what makes an array of them. A plain `[package]` may not
//! repeat, so a manifest typo is still caught.

use std::collections::BTreeMap;

/// The largest manifest the parser accepts.
///
/// A manifest is a description, not a program: 1 MiB is four orders of
/// magnitude more than any real one, and the cap is what makes "reject a huge
/// file" a diagnostic rather than a slow parse.
pub const MAX_MANIFEST_BYTES: usize = 1024 * 1024;

/// A failure with a 1-based line and column.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    pub line: usize,
    pub column: usize,
    pub message: String,
}

impl ParseError {
    /// The stable code a diagnostic for this error carries.
    pub fn code(&self) -> &'static str {
        yashy_common::codes::MANIFEST_SYNTAX
    }

    fn new(line: usize, column: usize, message: impl Into<String>) -> ParseError {
        ParseError {
            line,
            column,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}: {}", self.line, self.column, self.message)
    }
}

/// One value in the subset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    String(String),
    Integer(i64),
    Bool(bool),
    Array(Vec<Value>),
    Table(Table),
}

/// A table, ordered by key so every error and every rendering is
/// deterministic.
pub type Table = BTreeMap<String, Value>;

/// A 1-based position in the document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Location {
    pub line: usize,
    pub column: usize,
}

/// A parsed document: the root table plus where each key was written.
///
/// The manifest checks are semantic — a missing `version`, an unknown table —
/// and a reader that can only say "line 1" is a reader that cannot point at
/// the mistake. The spans ride along with the parse so the manifest layer can
/// report the line the author actually wrote.
#[derive(Clone, Debug)]
pub struct Document {
    pub root: Table,
    /// Dotted key path (`package`, `dependencies.http.path`) to its location.
    pub spans: BTreeMap<String, Location>,
}

impl Document {
    pub fn location(&self, path: &str) -> Location {
        self.spans.get(path).copied().unwrap_or(Location {
            line: 1,
            column: 1,
        })
    }

    fn set_location(&mut self, path: &str, line: usize, column: usize) {
        self.spans.insert(path.to_string(), Location { line, column });
    }
}

impl Value {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(text) => Some(text),
            _ => None,
        }
    }

    pub fn as_table(&self) -> Option<&Table> {
        match self {
            Value::Table(table) => Some(table),
            _ => None,
        }
    }
}

impl std::ops::Index<&str> for Value {
    type Output = Value;

    /// Index a table value by key, for tests and for the order-sensitive
    /// walks over a parsed document.
    fn index(&self, key: &str) -> &Value {
        match self {
            Value::Table(table) => &table[key],
            other => panic!("cannot index {other:?} by `{key}`: it is not a table"),
        }
    }
}

/// Where a key sits: a table key, or one element of an array of tables.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Step {
    Key(String),
    Element { key: String, index: usize },
}

impl Step {
    fn render(&self) -> String {
        match self {
            Step::Key(key) => key.clone(),
            Step::Element { key, index } => format!("{key}.{index}"),
        }
    }
}

/// Parse a document into its root table.
pub fn parse(text: &str) -> Result<Table, ParseError> {
    Ok(parse_document(text)?.root)
}

/// Parse a document into its root table plus the location of every key.
pub fn parse_document(text: &str) -> Result<Document, ParseError> {
    if text.len() > MAX_MANIFEST_BYTES {
        return Err(ParseError::new(
            1,
            1,
            format!(
                "manifest is {} bytes; the limit is {MAX_MANIFEST_BYTES}",
                text.len()
            ),
        ));
    }
    let mut document = Document {
        root: Table::new(),
        spans: BTreeMap::new(),
    };
    let mut headers: BTreeMap<String, usize> = BTreeMap::new();
    let mut current: Vec<Step> = Vec::new();
    for (offset, raw) in text.lines().enumerate() {
        let line_number = offset + 1;
        let line = strip_comment(raw);
        let line = line.trim_end();
        let trimmed = line.trim_start();
        if trimmed.is_empty() {
            continue;
        }
        let indent = line.len() - trimmed.len();
        // `[[array of tables]]` must be checked before `[table]`, because the
        // first `[` belongs to both spellings.
        if let Some(rest) = trimmed.strip_prefix("[[") {
            let Some(inner) = rest.strip_suffix("]]") else {
                return Err(ParseError::new(
                    line_number,
                    indent + 1,
                    "an array-of-tables header must end with `]]`",
                ));
            };
            let inner = inner.trim();
            let path = parse_key_path(inner, line_number, indent + 3)?;
            if path.is_empty() {
                return Err(ParseError::new(
                    line_number,
                    indent + 3,
                    "an array-of-tables header needs a name",
                ));
            }
            let name = path.join(".");
            let (last, parents) = path
                .split_last()
                .expect("the key path is not empty, checked above");
            let parent_path: Vec<Step> = parents.iter().cloned().map(Step::Key).collect();
            let parent = table_mut(&mut document.root, &parent_path, line_number, indent + 3)?;
            let index = match parent.get_mut(last) {
                None => {
                    parent.insert(last.clone(), Value::Array(vec![Value::Table(Table::new())]));
                    0
                }
                Some(Value::Array(items)) => {
                    if !items.iter().all(|item| matches!(item, Value::Table(_))) {
                        return Err(ParseError::new(
                            line_number,
                            indent + 3,
                            format!("`[[{name}]]` is an array of tables, but `{last}` holds other values"),
                        ));
                    }
                    items.push(Value::Table(Table::new()));
                    items.len() - 1
                }
                Some(_) => {
                    return Err(ParseError::new(
                        line_number,
                        indent + 3,
                        format!("`{last}` is already a value, not a table"),
                    ))
                }
            };
            document.set_location(&format!("{name}.{index}"), line_number, indent + 3);
            let mut segments = parent_path;
            segments.push(Step::Element {
                key: last.clone(),
                index,
            });
            current = segments;
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix('[') {
            let Some(inner) = rest.strip_suffix(']') else {
                return Err(ParseError::new(
                    line_number,
                    indent + 1,
                    "a table header must end with `]`",
                ));
            };
            let inner = inner.trim();
            if inner.is_empty() {
                return Err(ParseError::new(
                    line_number,
                    indent + 2,
                    "a table header needs a name",
                ));
            }
            let path = parse_key_path(inner, line_number, indent + 2)?;
            if path.is_empty() {
                return Err(ParseError::new(
                    line_number,
                    indent + 2,
                    "a table header needs a name",
                ));
            }
            let name = path.join(".");
            if let Some(first) = headers.insert(name.clone(), line_number) {
                return Err(ParseError::new(
                    line_number,
                    indent + 2,
                    format!("`[{name}]` is already defined on line {first}"),
                ));
            }
            let segments: Vec<Step> = path.iter().cloned().map(Step::Key).collect();
            table_mut(&mut document.root, &segments, line_number, indent + 2)?;
            document.set_location(&name, line_number, indent + 2);
            current = segments;
            continue;
        }
        if trimmed.starts_with(']') {
            return Err(ParseError::new(
                line_number,
                indent + 1,
                "unexpected `]`; a table header starts with `[`",
            ));
        }
        // `key = value`
        let Some(equals) = find_unquoted(trimmed, '=') else {
            return Err(ParseError::new(
                line_number,
                indent + 1,
                "expected `key = value` or a `[table]` header",
            ));
        };
        let key_text = trimmed[..equals].trim();
        if key_text.is_empty() {
            return Err(ParseError::new(
                line_number,
                indent + 1,
                "expected a key before `=`",
            ));
        }
        let path = parse_key_path(key_text, line_number, indent + 1)?;
        let value_start = indent + equals + 2;
        let value_text = trimmed[equals + 1..].trim();
        if value_text.is_empty() {
            return Err(ParseError::new(
                line_number,
                value_start,
                "expected a value after `=`",
            ));
        }
        let lead = trimmed[equals + 1..].len() - trimmed[equals + 1..].trim_start().len();
        let column = indent + equals + 1 + lead + 1;
        let value = parse_value(value_text, line_number, column)?;
        let mut full = current.clone();
        full.extend(path.iter().cloned().map(Step::Key));
        insert_value(&mut document.root, &full, value, line_number, column)?;
        let dotted = full.iter().map(Step::render).collect::<Vec<_>>().join(".");
        document.set_location(&dotted, line_number, column);
    }
    Ok(document)
}

/// Strip a `#` comment that is not inside a string.
fn strip_comment(line: &str) -> &str {
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (index, ch) in line.char_indices() {
        match quote {
            Some(_) if escaped => escaped = false,
            Some('"') if ch == '\\' => escaped = true,
            Some(q) if ch == q => quote = None,
            Some(_) => {}
            None if ch == '"' || ch == '\'' => quote = Some(ch),
            None if ch == '#' => return &line[..index],
            None => {}
        }
    }
    line
}

fn find_unquoted(text: &str, needle: char) -> Option<usize> {
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (index, ch) in text.char_indices() {
        match quote {
            Some(_) if escaped => escaped = false,
            Some('"') if ch == '\\' => escaped = true,
            Some(q) if ch == q => quote = None,
            Some(_) => {}
            None if ch == '"' || ch == '\'' => quote = Some(ch),
            None if ch == needle => return Some(index),
            None => {}
        }
    }
    None
}

fn parse_key_path(text: &str, line: usize, column: usize) -> Result<Vec<String>, ParseError> {
    let mut parts = Vec::new();
    let mut at = column;
    for piece in split_unquoted(text, '.') {
        let piece = piece.trim();
        if piece.is_empty() {
            return Err(ParseError::new(line, at, "a key segment cannot be empty"));
        }
        let key = if let Some(rest) = piece.strip_prefix('"') {
            let Some(inner) = rest.strip_suffix('"') else {
                return Err(ParseError::new(line, at, "an unterminated quoted key"));
            };
            unescape(inner, line, at)?
        } else if let Some(rest) = piece.strip_prefix('\'') {
            let Some(inner) = rest.strip_suffix('\'') else {
                return Err(ParseError::new(line, at, "an unterminated quoted key"));
            };
            inner.to_string()
        } else {
            if !piece
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
            {
                return Err(ParseError::new(
                    line,
                    at,
                    format!(
                        "`{piece}` is not a bare key; keys are letters, digits, `_` and `-`, \
                         or a quoted string"
                    ),
                ));
            }
            piece.to_string()
        };
        if key.is_empty() {
            return Err(ParseError::new(line, at, "a key segment cannot be empty"));
        }
        parts.push(key);
        at += piece.len() + 1;
    }
    Ok(parts)
}

fn split_unquoted(text: &str, separator: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut start = 0;
    for (index, ch) in text.char_indices() {
        match quote {
            Some(_) if escaped => escaped = false,
            Some('"') if ch == '\\' => escaped = true,
            Some(q) if ch == q => quote = None,
            Some(_) => {}
            None if ch == '"' || ch == '\'' => quote = Some(ch),
            None if ch == separator => {
                parts.push(&text[start..index]);
                start = index + ch.len_utf8();
            }
            None => {}
        }
    }
    parts.push(&text[start..]);
    parts
}

fn parse_value(text: &str, line: usize, column: usize) -> Result<Value, ParseError> {
    if let Some(rest) = text.strip_prefix('"') {
        let Some(inner) = rest.strip_suffix('"') else {
            return Err(ParseError::new(
                line,
                column,
                "an unterminated string; strings are single-line",
            ));
        };
        return Ok(Value::String(unescape(inner, line, column)?));
    }
    if let Some(rest) = text.strip_prefix('\'') {
        let Some(inner) = rest.strip_suffix('\'') else {
            return Err(ParseError::new(
                line,
                column,
                "an unterminated literal string; strings are single-line",
            ));
        };
        return Ok(Value::String(inner.to_string()));
    }
    if let Some(rest) = text.strip_prefix('[') {
        let Some(inner) = rest.strip_suffix(']') else {
            return Err(ParseError::new(
                line,
                column,
                "an array must close with `]` on the same line",
            ));
        };
        return parse_array(inner, line, column + 1);
    }
    match text {
        "true" => return Ok(Value::Bool(true)),
        "false" => return Ok(Value::Bool(false)),
        _ => {}
    }
    if let Ok(number) = text.parse::<i64>() {
        return Ok(Value::Integer(number));
    }
    Err(ParseError::new(
        line,
        column,
        format!(
            "`{text}` is not a value in the subset: strings, integers, booleans and arrays \
             of those"
        ),
    ))
}

fn parse_array(inner: &str, line: usize, column: usize) -> Result<Value, ParseError> {
    let mut items = Vec::new();
    let mut at = column;
    let mut rest = inner;
    loop {
        let trimmed = rest.trim_start();
        let skipped = rest.len() - trimmed.len();
        at += skipped;
        if trimmed.is_empty() {
            break;
        }
        if let Some(after) = trimmed.strip_prefix(',') {
            rest = after;
            at += 1;
            continue;
        }
        let Some(comma) = find_unquoted(trimmed, ',') else {
            items.push(parse_value(trimmed.trim_end(), line, at)?);
            break;
        };
        let item = trimmed[..comma].trim_end();
        if item.is_empty() {
            return Err(ParseError::new(line, at, "an array element cannot be empty"));
        }
        items.push(parse_value(item, line, at)?);
        rest = &trimmed[comma + 1..];
        at += comma + 1;
    }
    Ok(Value::Array(items))
}

fn unescape(text: &str, line: usize, column: usize) -> Result<String, ParseError> {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some('\\') => out.push('\\'),
            Some('"') => out.push('"'),
            Some(other) => {
                return Err(ParseError::new(
                    line,
                    column,
                    format!("`\\{other}` is not an escape in the subset (\\n, \\r, \\t, \\\\, \\\")"),
                ))
            }
            None => {
                return Err(ParseError::new(
                    line,
                    column,
                    "a trailing `\\` does not finish an escape",
                ))
            }
        }
    }
    Ok(out)
}

/// Walk to (and create, for plain keys) the table at `path`.
fn table_mut<'a>(
    root: &'a mut Table,
    path: &[Step],
    line: usize,
    column: usize,
) -> Result<&'a mut Table, ParseError> {
    let mut table = root;
    for step in path {
        match step {
            Step::Key(key) => {
                let entry = table
                    .entry(key.clone())
                    .or_insert_with(|| Value::Table(Table::new()));
                match entry {
                    Value::Table(next) => table = next,
                    other => {
                        return Err(ParseError::new(
                            line,
                            column,
                            format!("`{key}` is already a value, not a table ({other:?})"),
                        ))
                    }
                }
            }
            Step::Element { key, index } => {
                let Some(Value::Array(items)) = table.get_mut(key) else {
                    return Err(ParseError::new(
                        line,
                        column,
                        format!("`{key}` is not an array of tables"),
                    ));
                };
                match items.get_mut(*index) {
                    Some(Value::Table(next)) => table = next,
                    _ => {
                        return Err(ParseError::new(
                            line,
                            column,
                            format!("`{key}` has no table at index {index}"),
                        ))
                    }
                }
            }
        }
    }
    Ok(table)
}

fn insert_value(
    root: &mut Table,
    path: &[Step],
    value: Value,
    line: usize,
    column: usize,
) -> Result<(), ParseError> {
    let (last, parents) = path
        .split_last()
        .ok_or_else(|| ParseError::new(line, column, "expected a key"))?;
    let Step::Key(last) = last else {
        return Err(ParseError::new(line, column, "expected a key"));
    };
    let table = table_mut(root, parents, line, column)?;
    if table.contains_key(last) {
        return Err(ParseError::new(
            line,
            column,
            format!("`{last}` is assigned twice in one table"),
        ));
    }
    table.insert(last.clone(), value);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_subset_round_trips_through_the_model() {
        let text = r#"
# a comment
[package]
name = "demo"
version = "1.2.3"
cc = false
n = -4
deps = ["a", "b",]

[package.meta]
note = 'literal \n stays'
"#;
        let table = parse(text).expect("the sample parses");
        assert_eq!(
            table["package"]["name"],
            Value::String("demo".to_string())
        );
        assert_eq!(table["package"]["n"], Value::Integer(-4));
        assert_eq!(table["package"]["cc"], Value::Bool(false));
        assert_eq!(
            table["package"]["deps"],
            Value::Array(vec![
                Value::String("a".into()),
                Value::String("b".into())
            ])
        );
        assert_eq!(
            table["package"]["meta"]["note"],
            Value::String("literal \\n stays".to_string())
        );
    }

    #[test]
    fn a_comment_after_a_value_is_not_part_of_it() {
        let table = parse("a = \"x#y\" # tail\n").expect("parses");
        assert_eq!(table["a"], Value::String("x#y".to_string()));
    }

    #[test]
    fn a_huge_manifest_is_rejected_with_the_cap() {
        let text = "a".repeat(MAX_MANIFEST_BYTES + 1);
        let error = parse(&text).expect_err("over the cap");
        assert!(error.message.contains("limit"));
        assert_eq!(error.line, 1);
    }

    #[test]
    fn array_tables_append_and_keep_their_order() {
        let text = "[[package]]\nname = \"a\"\n\n[[package]]\nname = \"b\"\n";
        let table = parse(text).expect("the array parses");
        let Value::Array(packages) = &table["package"] else {
            panic!("package is an array of tables");
        };
        assert_eq!(packages.len(), 2);
        assert_eq!(packages[0]["name"], Value::String("a".to_string()));
        assert_eq!(packages[1]["name"], Value::String("b".to_string()));
    }

    #[test]
    fn every_malformed_shape_is_an_error_not_a_panic() {
        for text in [
            "[package",
            "[[package]",
            "[[package]]]",
            "a =",
            "a = ",
            "= 1",
            "a = \"x",
            "a = 'x",
            "a = [1, 2",
            "a = { b = 1 }",
            "a = 2020-01-01",
            "a = 1.5",
            "a = \"\\q\"",
            "[]",
            "a = 1\na = 2",
            "[a]\n[a]",
            "a.b = 1\na = 2",
            "a = [1]\n[[a]]",
        ] {
            assert!(parse(text).is_err(), "`{text}` must be an error");
        }
    }

    #[test]
    fn the_first_duplicate_is_reported_at_its_line() {
        let error = parse("a = 1\nb = 2\na = 3\n").expect_err("duplicate");
        assert_eq!(error.line, 3);
        assert!(error.message.contains('a'));
    }
}
