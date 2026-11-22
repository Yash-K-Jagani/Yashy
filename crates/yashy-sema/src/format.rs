//! The grammar of a literal `format` specifier.
//!
//! The checker parses the string when `format("...", ...)` is checked, so a
//! malformed field, a wrong argument count or a type mismatch is a diagnostic
//! at the call site rather than a surprise at run time. Only the *shape* is
//! decided here; the checker owns the types and the lowering.

/// How a field is placed in its width.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Right,
    Center,
}

impl Align {
    /// The backend's alignment code, shared with `str_pad`.
    pub fn code(self) -> i64 {
        match self {
            Align::Left => 0,
            Align::Right => 1,
            Align::Center => 2,
        }
    }
}

/// What kind of value a field asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wanted {
    /// Any printable value; the checker picks the conversion from its type.
    Auto,
    /// An `int`.
    Int,
    /// A `float`.
    Float,
    /// A `string`.
    Str,
    /// Any value with a debug form (`{:?}`).
    Debug,
}

/// One replacement field: `{:>8.2}`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    pub fill: String,
    pub align: Align,
    pub zero: bool,
    pub width: i64,
    pub precision: Option<i64>,
    pub wanted: Wanted,
}

impl Default for Field {
    fn default() -> Self {
        Field {
            fill: " ".to_string(),
            align: Align::Left,
            zero: false,
            width: 0,
            precision: None,
            wanted: Wanted::Auto,
        }
    }
}

/// A decoded piece of the format string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Piece {
    Text(String),
    Field(Field),
}

/// Parse `spec` into text and fields.
///
/// The grammar is `[[fill]align][0][width][.precision][type]`, where `align`
/// is `<`, `>` or `^` and `type` is `d`, `f`, `s` or `?`. `{{` and `}}` are
/// literal braces. An error names the offending byte offset in the spec.
pub fn parse(spec: &str) -> Result<Vec<Piece>, String> {
    let chars: Vec<char> = spec.chars().collect();
    let mut pieces: Vec<Piece> = Vec::new();
    let mut text = String::new();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '{' if i + 1 < chars.len() && chars[i + 1] == '{' => {
                text.push('{');
                i += 2;
            }
            '}' if i + 1 < chars.len() && chars[i + 1] == '}' => {
                text.push('}');
                i += 2;
            }
            '{' => {
                let start = i;
                i += 1;
                // `{}` takes the defaults; `{:...}` introduces a specifier.
                if i < chars.len() && chars[i] == ':' {
                    i += 1;
                }
                let mut field = Field::default();
                // `[fill]align`: an align in position 0 uses the default fill;
                // otherwise the preceding scalar is the fill.
                if i < chars.len() && is_align(chars[i]) {
                    field.align = align_of(chars[i]);
                    i += 1;
                } else if i + 1 < chars.len() && is_align(chars[i + 1]) {
                    field.fill = chars[i].to_string();
                    field.align = align_of(chars[i + 1]);
                    i += 2;
                }
                if i < chars.len() && chars[i] == '0' {
                    field.zero = true;
                    field.fill = "0".to_string();
                    field.align = Align::Right;
                    i += 1;
                }
                let width_start = i;
                while i < chars.len() && chars[i].is_ascii_digit() {
                    i += 1;
                }
                if i > width_start {
                    field.width = digits(&chars[width_start..i], start, "width")?;
                    if field.width > 4096 {
                        return Err(format!(
                            "invalid format specifier at byte {}: width is larger than 4096",
                            byte_offset(spec, start)
                        ));
                    }
                }
                if i < chars.len() && chars[i] == '.' {
                    i += 1;
                    let prec_start = i;
                    while i < chars.len() && chars[i].is_ascii_digit() {
                        i += 1;
                    }
                    if i == prec_start {
                        return Err(format!(
                            "invalid format specifier at byte {}: `.` needs a precision",
                            byte_offset(spec, start)
                        ));
                    }
                    field.precision = Some(digits(&chars[prec_start..i], start, "precision")?);
                    if field.precision.is_some_and(|precision| precision > 17) {
                        return Err(format!(
                            "invalid format specifier at byte {}: precision is larger than 17",
                            byte_offset(spec, start)
                        ));
                    }
                }
                if i < chars.len() {
                    field.wanted = match chars[i] {
                        'd' => Wanted::Int,
                        'f' => Wanted::Float,
                        's' => Wanted::Str,
                        '?' => Wanted::Debug,
                        '}' => Wanted::Auto,
                        other => {
                            return Err(format!(
                                "invalid format specifier at byte {}: unknown type `{other}`",
                                byte_offset(spec, i)
                            ))
                        }
                    };
                    if field.wanted != Wanted::Auto {
                        i += 1;
                    }
                }
                if i >= chars.len() || chars[i] != '}' {
                    return Err(format!(
                        "invalid format specifier at byte {}: expected `}}`",
                        byte_offset(spec, start)
                    ));
                }
                i += 1;
                if !text.is_empty() {
                    pieces.push(Piece::Text(std::mem::take(&mut text)));
                }
                pieces.push(Piece::Field(field));
            }
            '}' => {
                return Err(format!(
                    "invalid format specifier at byte {}: unmatched `}}`",
                    byte_offset(spec, i)
                ))
            }
            other => {
                text.push(other);
                i += 1;
            }
        }
    }
    if !text.is_empty() {
        pieces.push(Piece::Text(text));
    }
    Ok(pieces)
}

/// The number of fields a spec consumes.
pub fn field_count(pieces: &[Piece]) -> usize {
    pieces
        .iter()
        .filter(|piece| matches!(piece, Piece::Field(_)))
        .count()
}

fn is_align(ch: char) -> bool {
    matches!(ch, '<' | '>' | '^')
}

fn align_of(ch: char) -> Align {
    match ch {
        '<' => Align::Left,
        '>' => Align::Right,
        _ => Align::Center,
    }
}

fn digits(chars: &[char], start: usize, what: &str) -> Result<i64, String> {
    let text: String = chars.iter().collect();
    text.parse::<i64>()
        .map_err(|_| format!("invalid format specifier at byte {start}: {what} is too large"))
}

fn byte_offset(spec: &str, char_index: usize) -> usize {
    spec.char_indices()
        .nth(char_index)
        .map(|(byte, _)| byte)
        .unwrap_or(spec.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_plan_example() {
        assert_eq!(
            parse("{:>8.2}").expect("parses"),
            vec![Piece::Field(Field {
                align: Align::Right,
                width: 8,
                precision: Some(2),
                ..Field::default()
            })]
        );
    }

    #[test]
    fn parses_fill_zero_and_escapes() {
        assert_eq!(
            parse("{{x}} {:*^5d}").expect("parses"),
            vec![
                Piece::Text("{x} ".to_string()),
                Piece::Field(Field {
                    fill: "*".to_string(),
                    align: Align::Center,
                    wanted: Wanted::Int,
                    width: 5,
                    ..Field::default()
                }),
            ]
        );
    }

    #[test]
    fn rejects_bad_specs() {
        assert!(parse("{:z}").is_err());
        assert!(parse("{:>}x}").is_err());
        assert!(parse("{:.}").is_err());
        assert!(parse("}").is_err());
    }
}
