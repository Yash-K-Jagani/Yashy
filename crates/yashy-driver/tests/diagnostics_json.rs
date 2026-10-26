//! Schema-v1 tests for `yashy check --json` and `yashy test --json`.
//!
//! The tests read the output with a tiny JSON parser instead of matching
//! message text. The schema is a contract with tools, so the shape and the
//! facts (code, severity, file, span, summary) are what matter; the message is
//! checked only for agreement between the human and JSON surfaces.

mod common;

use common::{work_dir, write, yashy};
use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn run(dir: &Path, args: &[&str]) -> Output {
    Command::new(yashy())
        .args(args)
        .current_dir(dir)
        .output()
        .expect("the yashy CLI must be runnable")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn check_json_reports_a_coded_diagnostic() {
    let dir = work_dir("json-check-error");
    let source_text = "fn main() {\n    const count: int = \"three\";\n}\n";
    let source = dir.join("bad.ya");
    write(&dir, "bad.ya", source_text);

    let output = run(&dir, &["check", "--json", source.to_str().unwrap()]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "a type error must fail:\n{}",
        stderr(&output)
    );
    let report = Json::parse(&stdout(&output));

    assert_eq!(report.get("schema_version").as_i64(), 1);
    assert_eq!(report.get("tool").as_str(), "check");
    assert!(!report.get("yashy_version").as_str().is_empty());
    assert!(!report.get("success").as_bool());

    let diagnostics = report.get("diagnostics").as_array();
    assert_eq!(diagnostics.len(), 1);
    let diagnostic = &diagnostics[0];
    assert_eq!(diagnostic.get("code").as_str(), "E0100");
    assert_eq!(diagnostic.get("severity").as_str(), "error");
    assert!(!diagnostic.get("message").as_str().is_empty());
    assert_eq!(diagnostic.get("file").as_str(), source.to_str().unwrap());

    let literal = "\"three\"";
    let start = source_text
        .find(literal)
        .expect("the literal is in the source") as i64;
    let end = start + literal.len() as i64;
    let span = diagnostic.get("span");
    assert_eq!(span.get("start").as_i64(), start);
    assert_eq!(span.get("end").as_i64(), end);

    let line_start = source_text[..start as usize]
        .rfind('\n')
        .map(|index| index + 1)
        .unwrap_or(0);
    let line = source_text[..start as usize].matches('\n').count() as i64 + 1;
    let column = start - line_start as i64 + 1;
    assert_eq!(span.get("line").as_i64(), line);
    assert_eq!(span.get("column").as_i64(), column);

    assert_eq!(
        diagnostic.get("docs_url").as_str(),
        "https://yashy.dev/errors/E0100"
    );

    let summary = report.get("summary");
    assert_eq!(summary.get("errors").as_i64(), 1);
    assert_eq!(summary.get("warnings").as_i64(), 0);
}

#[test]
fn check_json_reports_success_with_no_diagnostics() {
    let dir = work_dir("json-check-ok");
    write(&dir, "good.ya", "fn main() {}\n");
    let source = dir.join("good.ya");

    let output = run(&dir, &["check", "--json", source.to_str().unwrap()]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "a clean file must succeed:\n{}",
        stderr(&output)
    );
    let report = Json::parse(&stdout(&output));

    assert!(report.get("success").as_bool());
    assert!(report.get("diagnostics").as_array().is_empty());
    assert_eq!(report.get("summary").get("errors").as_i64(), 0);
    assert_eq!(report.get("summary").get("warnings").as_i64(), 0);
}

#[test]
fn human_and_json_agree_on_the_diagnostic_facts() {
    let dir = work_dir("json-human-agreement");
    let source_text = "fn main() {\n    const count: int = \"three\";\n}\n";
    let source = dir.join("bad.ya");
    write(&dir, "bad.ya", source_text);

    let human = run(&dir, &["check", source.to_str().unwrap()]);
    let json = run(&dir, &["check", "--json", source.to_str().unwrap()]);
    assert_eq!(human.status.code(), Some(1));
    assert_eq!(json.status.code(), Some(1));

    let human_text = stderr(&human);
    let mut lines = human_text.lines();
    let location = lines.next().expect("the location line");
    let severity = lines.next().expect("the severity line");

    let (file, line, column) = split_location(location);
    let (code, message) = split_severity(severity);

    let report = Json::parse(&stdout(&json));
    let diagnostics = report.get("diagnostics").as_array();
    assert_eq!(diagnostics.len(), 1);
    let diagnostic = &diagnostics[0];
    assert_eq!(diagnostic.get("code").as_str(), code);
    assert_eq!(diagnostic.get("severity").as_str(), "error");
    assert_eq!(diagnostic.get("message").as_str(), message);
    assert_eq!(diagnostic.get("file").as_str(), file);
    assert_eq!(diagnostic.get("span").get("line").as_i64(), line);
    assert_eq!(diagnostic.get("span").get("column").as_i64(), column);
    assert_eq!(report.get("summary").get("errors").as_i64(), 1);
}

#[test]
fn check_usage_errors_exit_2() {
    let dir = work_dir("json-check-usage");
    let missing = dir.join("missing.ya");

    let no_input = run(&dir, &["check", "--json"]);
    assert_eq!(no_input.status.code(), Some(2));
    assert!(stderr(&no_input).contains("needs a source file"));

    let unknown = run(&dir, &["check", "--json", "--bogus"]);
    assert_eq!(unknown.status.code(), Some(2));
    assert!(stderr(&unknown).contains("unknown option"));

    let no_file = run(&dir, &["check", "--json", missing.to_str().unwrap()]);
    assert_eq!(no_file.status.code(), Some(2));
    assert!(stderr(&no_file).contains("cannot read"));
}

#[test]
fn test_json_reports_each_test_and_the_summary() {
    let dir = work_dir("json-test-mixed");
    write(&dir, "tests/pass.ya", "fn main() { assert(1 + 1 == 2); }\n");
    write(&dir, "tests/bad.ya", "fn main() {\n    missing();\n}\n");

    let output = run(&dir, &["test", "--json"]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "a failing test must fail the run:\n{}",
        stderr(&output)
    );
    let report = Json::parse(&stdout(&output));

    assert_eq!(report.get("schema_version").as_i64(), 1);
    assert_eq!(report.get("tool").as_str(), "test");
    assert!(!report.get("success").as_bool());
    assert!(report.get("diagnostics").as_array().is_empty());

    let tests = report.get("tests").as_array();
    assert_eq!(tests.len(), 2);

    let passed = tests
        .iter()
        .find(|test| test.get("file").as_str() == "tests/pass.ya")
        .expect("the passing test");
    assert_eq!(passed.get("name").as_str(), "pass");
    assert_eq!(passed.get("status").as_str(), "passed");
    assert!(passed.get("diagnostics").as_array().is_empty());
    assert!(passed.get("duration_ms").as_i64() >= 0);

    let failed = tests
        .iter()
        .find(|test| test.get("file").as_str() == "tests/bad.ya")
        .expect("the failing test");
    assert_eq!(failed.get("name").as_str(), "bad");
    assert_eq!(failed.get("status").as_str(), "failed");
    let diagnostics = failed.get("diagnostics").as_array();
    assert_eq!(diagnostics.len(), 1);
    let diagnostic = &diagnostics[0];
    assert_eq!(diagnostic.get("code").as_str(), "E0001");
    assert_eq!(diagnostic.get("severity").as_str(), "error");
    assert_eq!(diagnostic.get("file").as_str(), "tests/bad.ya");
    assert_eq!(diagnostic.get("span").get("line").as_i64(), 2);

    let summary = report.get("summary");
    assert_eq!(summary.get("passed").as_i64(), 1);
    assert_eq!(summary.get("failed").as_i64(), 1);
    assert_eq!(summary.get("skipped").as_i64(), 0);
}

#[test]
fn test_json_all_pass_exits_zero() {
    let dir = work_dir("json-test-ok");
    write(&dir, "tests/pass.ya", "fn main() { assert(1 + 1 == 2); }\n");

    let output = run(&dir, &["test", "--json"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "a passing run must succeed:\n{}",
        stderr(&output)
    );
    let report = Json::parse(&stdout(&output));

    assert!(report.get("success").as_bool());
    assert_eq!(report.get("tests").as_array().len(), 1);
    let summary = report.get("summary");
    assert_eq!(summary.get("passed").as_i64(), 1);
    assert_eq!(summary.get("failed").as_i64(), 0);
    assert_eq!(summary.get("skipped").as_i64(), 0);
}

#[test]
fn test_json_usage_errors_exit_2() {
    let dir = work_dir("json-test-usage");

    let list = run(&dir, &["test", "--json", "--list"]);
    assert_eq!(list.status.code(), Some(2));
    assert!(stderr(&list).contains("cannot be combined"));

    let missing = run(&dir, &["test", "--json", "no-such-directory"]);
    assert_eq!(missing.status.code(), Some(2));
    assert!(stderr(&missing).contains("no tests at"));
}

#[test]
fn test_json_internal_error_exits_3() {
    let dir = work_dir("json-test-internal");
    write(&dir, "tests/pass.ya", "fn main() {}\n");
    let not_a_dir = dir.join("not-a-dir");
    fs::write(&not_a_dir, "a file where a directory is needed").expect("the fixture is writable");

    let output = Command::new(yashy())
        .args(["test", "--json"])
        .current_dir(&dir)
        .env("TMPDIR", &not_a_dir)
        .env("TMP", &not_a_dir)
        .env("TEMP", &not_a_dir)
        .output()
        .expect("the yashy CLI must be runnable");

    assert_eq!(
        output.status.code(),
        Some(3),
        "a directory that cannot be created is an internal error:\n{}",
        stderr(&output)
    );
    let report = Json::parse(&stdout(&output));
    assert!(!report.get("success").as_bool());
    let diagnostics = report.get("diagnostics").as_array();
    assert_eq!(diagnostics.len(), 1);
    assert!(diagnostics[0]
        .get("message")
        .as_str()
        .contains("cannot create"));
    assert!(report.get("tests").as_array().is_empty());
}

/// Splits the renderer's `path:line:column` line. `rsplitn` keeps a Windows
/// drive letter (or any other colon in the path) in the file component.
fn split_location(line: &str) -> (&str, i64, i64) {
    let mut parts = line.rsplitn(3, ':');
    let column = parts
        .next()
        .expect("a column")
        .parse()
        .expect("a numeric column");
    let line_number = parts
        .next()
        .expect("a line")
        .parse()
        .expect("a numeric line");
    let file = parts.next().expect("a file");
    (file, line_number, column)
}

/// Splits `error[E0100]: message` into the code and the message.
fn split_severity(line: &str) -> (&str, &str) {
    let (label, message) = line.split_once(": ").expect("severity and message");
    let code = label
        .strip_prefix("error[")
        .and_then(|rest| rest.strip_suffix(']'))
        .expect("a coded error");
    (code, message)
}

/// A JSON value, just large enough to read the schema the CLI emits.
#[derive(Clone, Debug, PartialEq)]
enum Json {
    Null,
    Bool(bool),
    Number(i64),
    String(String),
    Array(Vec<Json>),
    Object(Vec<(String, Json)>),
}

impl Json {
    fn parse(text: &str) -> Json {
        let mut parser = Parser {
            bytes: text.as_bytes(),
            pos: 0,
        };
        let value = parser.value();
        parser.skip_whitespace();
        assert_eq!(
            parser.pos,
            parser.bytes.len(),
            "trailing bytes in JSON: {:?}",
            &text[parser.pos..]
        );
        value
    }

    fn get(&self, key: &str) -> &Json {
        match self {
            Json::Object(fields) => fields
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value)
                .unwrap_or_else(|| panic!("missing key `{key}` in {self:?}")),
            other => panic!("expected an object for key `{key}`, found {other:?}"),
        }
    }

    fn as_str(&self) -> &str {
        match self {
            Json::String(value) => value,
            other => panic!("expected a string, found {other:?}"),
        }
    }

    fn as_bool(&self) -> bool {
        match self {
            Json::Bool(value) => *value,
            other => panic!("expected a boolean, found {other:?}"),
        }
    }

    fn as_i64(&self) -> i64 {
        match self {
            Json::Number(value) => *value,
            other => panic!("expected a number, found {other:?}"),
        }
    }

    fn as_array(&self) -> &[Json] {
        match self {
            Json::Array(values) => values,
            other => panic!("expected an array, found {other:?}"),
        }
    }
}

struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn value(&mut self) -> Json {
        self.skip_whitespace();
        match self.bytes.get(self.pos) {
            Some(b'{') => self.object(),
            Some(b'[') => self.array(),
            Some(b'"') => Json::String(self.string()),
            Some(b't') => {
                self.expect_word("true");
                Json::Bool(true)
            }
            Some(b'f') => {
                self.expect_word("false");
                Json::Bool(false)
            }
            Some(b'n') => {
                self.expect_word("null");
                Json::Null
            }
            Some(byte) if *byte == b'-' || byte.is_ascii_digit() => self.number(),
            Some(byte) => panic!("unexpected byte {:?} at {}", *byte as char, self.pos),
            None => panic!("unexpected end of JSON"),
        }
    }

    fn object(&mut self) -> Json {
        self.pos += 1;
        let mut fields = Vec::new();
        self.skip_whitespace();
        if self.bytes.get(self.pos) == Some(&b'}') {
            self.pos += 1;
            return Json::Object(fields);
        }
        loop {
            self.skip_whitespace();
            let key = self.string();
            self.skip_whitespace();
            assert_eq!(
                self.bytes.get(self.pos),
                Some(&b':'),
                "expected `:` at {}",
                self.pos
            );
            self.pos += 1;
            let value = self.value();
            fields.push((key, value));
            self.skip_whitespace();
            match self.bytes.get(self.pos) {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    break;
                }
                other => panic!("expected `,` or `}}` at {}, found {other:?}", self.pos),
            }
        }
        Json::Object(fields)
    }

    fn array(&mut self) -> Json {
        self.pos += 1;
        let mut values = Vec::new();
        self.skip_whitespace();
        if self.bytes.get(self.pos) == Some(&b']') {
            self.pos += 1;
            return Json::Array(values);
        }
        loop {
            values.push(self.value());
            self.skip_whitespace();
            match self.bytes.get(self.pos) {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    break;
                }
                other => panic!("expected `,` or `]` at {}, found {other:?}", self.pos),
            }
        }
        Json::Array(values)
    }

    fn string(&mut self) -> String {
        assert_eq!(
            self.bytes.get(self.pos),
            Some(&b'"'),
            "expected a string at {}",
            self.pos
        );
        self.pos += 1;
        let mut out = String::new();
        loop {
            let byte = *self
                .bytes
                .get(self.pos)
                .unwrap_or_else(|| panic!("unterminated string"));
            self.pos += 1;
            match byte {
                b'"' => break,
                b'\\' => {
                    let escape = *self
                        .bytes
                        .get(self.pos)
                        .unwrap_or_else(|| panic!("unterminated escape"));
                    self.pos += 1;
                    match escape {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{08}'),
                        b'f' => out.push('\u{0c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let hex = std::str::from_utf8(&self.bytes[self.pos..self.pos + 4])
                                .expect("four hex digits");
                            let code = u32::from_str_radix(hex, 16).expect("hex digits");
                            self.pos += 4;
                            out.push(char::from_u32(code).expect("a Unicode scalar"));
                        }
                        other => panic!("unknown escape `\\{}`", other as char),
                    }
                }
                byte if byte < 0x20 => panic!("control byte in a string"),
                byte if byte < 0x80 => out.push(byte as char),
                _ => {
                    let rest = std::str::from_utf8(&self.bytes[self.pos - 1..])
                        .expect("the string is valid UTF-8");
                    let character = rest.chars().next().expect("a character");
                    out.push(character);
                    self.pos += character.len_utf8() - 1;
                }
            }
        }
        out
    }

    fn number(&mut self) -> Json {
        let start = self.pos;
        if self.bytes.get(self.pos) == Some(&b'-') {
            self.pos += 1;
        }
        while let Some(byte) = self.bytes.get(self.pos) {
            if byte.is_ascii_digit() {
                self.pos += 1;
            } else {
                break;
            }
        }
        let text = std::str::from_utf8(&self.bytes[start..self.pos]).expect("a number");
        Json::Number(text.parse().expect("an integer"))
    }

    fn expect_word(&mut self, word: &str) {
        assert!(
            self.bytes[self.pos..].starts_with(word.as_bytes()),
            "expected `{word}` at {}",
            self.pos
        );
        self.pos += word.len();
    }

    fn skip_whitespace(&mut self) {
        while let Some(byte) = self.bytes.get(self.pos) {
            if matches!(byte, b' ' | b'\n' | b'\r' | b'\t') {
                self.pos += 1;
            } else {
                break;
            }
        }
    }
}
