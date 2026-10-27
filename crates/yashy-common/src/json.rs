//! A dependency-free JSON writer for the CLI's machine-readable reports.
//!
//! The compiler has one JSON surface (schema v1), so the escaping and the
//! layout live here rather than in each command. A value tree keeps the
//! construction readable and makes "omit an empty optional field" a property
//! of the caller's code rather than a string-building accident.

/// A JSON value.
#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    /// `null`.
    Null,
    /// `true` or `false`.
    Bool(bool),
    /// A signed integer. The schema has no fractional numbers.
    Number(i64),
    /// A string, escaped on output.
    String(String),
    /// An array, in insertion order.
    Array(Vec<Json>),
    /// An object, in insertion order.
    Object(Vec<(String, Json)>),
}

impl Json {
    /// `null`.
    pub fn null() -> Json {
        Json::Null
    }

    /// A boolean.
    pub fn bool(value: bool) -> Json {
        Json::Bool(value)
    }

    /// An integer.
    pub fn number(value: i64) -> Json {
        Json::Number(value)
    }

    /// A string.
    pub fn string(value: impl Into<String>) -> Json {
        Json::String(value.into())
    }

    /// An empty object.
    pub fn object() -> Json {
        Json::Object(Vec::new())
    }

    /// An array of `values`.
    pub fn array(values: Vec<Json>) -> Json {
        Json::Array(values)
    }

    /// Adds a field to an object.
    ///
    /// # Panics
    ///
    /// Panics when `self` is not an object. The writer is an internal tool:
    /// every call site builds a value of a known shape, so a wrong receiver is
    /// a programming error, not user input.
    pub fn set(&mut self, key: impl Into<String>, value: Json) -> &mut Json {
        match self {
            Json::Object(fields) => {
                fields.push((key.into(), value));
                self
            }
            _ => panic!("Json::set needs an object"),
        }
    }

    /// Appends a value to an array.
    ///
    /// # Panics
    ///
    /// Panics when `self` is not an array, for the same reason as [`Json::set`].
    pub fn push(&mut self, value: Json) -> &mut Json {
        match self {
            Json::Array(values) => {
                values.push(value);
                self
            }
            _ => panic!("Json::push needs an array"),
        }
    }

    /// The pretty-printed form used by the CLI: two-space indentation, one
    /// field or element per line, and no trailing newline.
    pub fn render(&self) -> String {
        let mut out = String::new();
        self.write(&mut out, 0);
        out
    }

    fn write(&self, out: &mut String, indent: usize) {
        match self {
            Json::Null => out.push_str("null"),
            Json::Bool(value) => out.push_str(if *value { "true" } else { "false" }),
            Json::Number(value) => out.push_str(&value.to_string()),
            Json::String(value) => write_string(out, value),
            Json::Array(values) => {
                if values.is_empty() {
                    out.push_str("[]");
                    return;
                }
                out.push_str("[\n");
                for (index, value) in values.iter().enumerate() {
                    if index > 0 {
                        out.push_str(",\n");
                    }
                    write_indent(out, indent + 1);
                    value.write(out, indent + 1);
                }
                out.push('\n');
                write_indent(out, indent);
                out.push(']');
            }
            Json::Object(fields) => {
                if fields.is_empty() {
                    out.push_str("{}");
                    return;
                }
                out.push_str("{\n");
                for (index, (key, value)) in fields.iter().enumerate() {
                    if index > 0 {
                        out.push_str(",\n");
                    }
                    write_indent(out, indent + 1);
                    write_string(out, key);
                    out.push_str(": ");
                    value.write(out, indent + 1);
                }
                out.push('\n');
                write_indent(out, indent);
                out.push('}');
            }
        }
    }
}

fn write_indent(out: &mut String, indent: usize) {
    for _ in 0..indent {
        out.push_str("  ");
    }
}

/// Appends `value` as a quoted JSON string.
fn write_string(out: &mut String, value: &str) {
    out.push('"');
    for character in value.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            character if (character as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", character as u32));
            }
            character => out.push(character),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::Json;

    #[test]
    fn scalars_render_as_json() {
        assert_eq!(Json::null().render(), "null");
        assert_eq!(Json::bool(true).render(), "true");
        assert_eq!(Json::bool(false).render(), "false");
        assert_eq!(Json::number(-7).render(), "-7");
        assert_eq!(Json::string("plain").render(), "\"plain\"");
    }

    #[test]
    fn strings_escape_the_characters_json_requires() {
        let rendered = Json::string("a\"b\\c\n\r\t\u{08}\u{0c}\u{1}").render();
        assert_eq!(rendered, "\"a\\\"b\\\\c\\n\\r\\t\\b\\f\\u0001\"");
    }

    #[test]
    fn unicode_is_kept_as_written() {
        assert_eq!(Json::string("héllo ✓").render(), "\"héllo ✓\"");
    }

    #[test]
    fn an_object_keeps_insertion_order_and_indents_by_two_spaces() {
        let mut object = Json::object();
        object.set("schema_version", Json::number(1));
        object.set("success", Json::bool(false));
        object.set("diagnostics", Json::array(vec![]));
        assert_eq!(
            object.render(),
            "{\n  \"schema_version\": 1,\n  \"success\": false,\n  \"diagnostics\": []\n}"
        );
    }

    #[test]
    fn a_nested_array_puts_one_element_on_each_line() {
        let mut object = Json::object();
        object.set(
            "tests",
            Json::array(vec![
                Json::string("one"),
                Json::array(vec![Json::number(2), Json::null()]),
            ]),
        );
        assert_eq!(
            object.render(),
            "{\n  \"tests\": [\n    \"one\",\n    [\n      2,\n      null\n    ]\n  ]\n}"
        );
    }

    #[test]
    fn empty_containers_stay_on_one_line() {
        assert_eq!(Json::object().render(), "{}");
        assert_eq!(Json::array(vec![]).render(), "[]");
        let mut object = Json::object();
        object.set("empty", Json::object());
        assert_eq!(object.render(), "{\n  \"empty\": {}\n}");
    }

    #[test]
    fn a_key_is_escaped_like_a_string() {
        let mut object = Json::object();
        object.set("a\"b", Json::number(1));
        assert_eq!(object.render(), "{\n  \"a\\\"b\": 1\n}");
    }

    #[test]
    fn set_and_push_build_nested_values() {
        let mut array = Json::array(vec![]);
        array.push(Json::number(1));
        array.push(Json::string("two"));
        let mut object = Json::object();
        object.set("values", array);
        assert_eq!(
            object.render(),
            "{\n  \"values\": [\n    1,\n    \"two\"\n  ]\n}"
        );
    }
}
