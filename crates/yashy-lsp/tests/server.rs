//! The language server, driven through an in-memory pipe.
//!
//! Each test writes a batch of framed messages and reads the framed replies,
//! which is the same bytes an editor would exchange over stdio.

use std::io::{Cursor, Write};
use std::path::PathBuf;

use yashy_lsp::json::{number, object, string, to_string, Value};
use yashy_lsp::path_to_uri;

fn request(id: i64, method: &str, params: Value) -> Value {
    object([
        ("jsonrpc", string("2.0")),
        ("id", number(id)),
        ("method", string(method)),
        ("params", params),
    ])
}

fn notification(method: &str, params: Value) -> Value {
    object([
        ("jsonrpc", string("2.0")),
        ("method", string(method)),
        ("params", params),
    ])
}

fn did_open(uri: &str, text: &str) -> Value {
    notification(
        "textDocument/didOpen",
        object([(
            "textDocument",
            object([
                ("uri", string(uri)),
                ("languageId", string("yashy")),
                ("version", number(1)),
                ("text", string(text)),
            ]),
        )]),
    )
}

fn did_change(uri: &str, version: i64, text: &str) -> Value {
    notification(
        "textDocument/didChange",
        object([
            (
                "textDocument",
                object([("uri", string(uri)), ("version", number(version))]),
            ),
            (
                "contentChanges",
                Value::Arr(vec![object([("text", string(text))])]),
            ),
        ]),
    )
}

fn position(line: i64, character: i64) -> Value {
    object([("line", number(line)), ("character", number(character))])
}

fn at(uri: &str, line: i64, character: i64) -> Value {
    object([
        (
            "textDocument",
            object([("uri", string(uri))]),
        ),
        ("position", position(line, character)),
    ])
}

/// Feed every message to a fresh server and return its replies in order.
fn run(messages: Vec<Value>) -> Vec<Value> {
    let mut input = Vec::new();
    for message in messages {
        let body = to_string(&message);
        write!(input, "Content-Length: {}\r\n\r\n{body}", body.len())
            .expect("writing to a Vec cannot fail");
    }
    let mut output = Vec::new();
    yashy_lsp::serve(Cursor::new(input), &mut output).expect("the server must not fail");
    parse_replies(&output)
}

fn parse_replies(output: &[u8]) -> Vec<Value> {
    let text = String::from_utf8(output.to_vec()).expect("the server writes UTF-8");
    let mut replies = Vec::new();
    let mut rest = text.as_str();
    while let Some(header_end) = rest.find("\r\n\r\n") {
        let header = &rest[..header_end];
        let length: usize = header
            .strip_prefix("Content-Length: ")
            .and_then(|value| value.trim().parse().ok())
            .expect("every reply has a Content-Length");
        let body_start = header_end + 4;
        let body = rest
            .get(body_start..body_start + length)
            .expect("the reply body is complete");
        replies.push(yashy_lsp::json::parse(body).expect("a reply must be JSON"));
        rest = rest.get(body_start + length..).unwrap_or("");
    }
    replies
}

fn publish(replies: &[Value]) -> Option<&Value> {
    replies
        .iter()
        .find(|reply| reply.get("method").and_then(Value::as_str) == Some("textDocument/publishDiagnostics"))
}

fn test_uri(name: &str) -> String {
    let dir = std::env::temp_dir().join(format!("yashy-lsp-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("the test directory must be creatable");
    path_to_uri(&dir.join(name))
}

#[test]
fn opening_a_file_with_an_error_publishes_a_diagnostic_with_its_range() {
    let uri = test_uri("error.ya");
    let messages = vec![
        request(1, "initialize", object([("capabilities", object([]))])),
        did_open(&uri, "fn main() {\n    missing();\n}\n"),
    ];
    let replies = run(messages);
    let published = publish(&replies).expect("a diagnostic must be published");
    let diagnostics = published
        .get_path(&["params", "diagnostics"])
        .and_then(Value::as_array)
        .expect("the notification carries diagnostics");
    assert!(!diagnostics.is_empty(), "the unknown call must be reported");
    let first = diagnostics.first().expect("one diagnostic");
    let line = first
        .get_path(&["range", "start", "line"])
        .and_then(Value::as_i64)
        .expect("the range has a start line");
    assert_eq!(
        line, 1,
        "the caret belongs on the call line; diagnostics: {diagnostics:?}"
    );
}

#[test]
fn fixing_the_error_clears_the_diagnostic() {
    let uri = test_uri("fixed.ya");
    let messages = vec![
        did_open(&uri, "fn main() {\n    missing();\n}\n"),
        did_change(&uri, 2, "fn main() {\n    print_str(\"ok\");\n}\n"),
    ];
    let replies = run(messages);
    let published: Vec<&Value> = replies
        .iter()
        .filter(|reply| {
            reply.get("method").and_then(Value::as_str)
                == Some("textDocument/publishDiagnostics")
        })
        .collect();
    assert_eq!(published.len(), 2, "one publish per change");
    let last = published.last().expect("the second publish");
    let diagnostics = last
        .get_path(&["params", "diagnostics"])
        .and_then(Value::as_array)
        .expect("the notification carries diagnostics");
    assert!(
        diagnostics.is_empty(),
        "the fixed file must publish no diagnostics"
    );
}

#[test]
fn hover_on_a_builtin_shows_its_signature() {
    let uri = test_uri("hover.ya");
    let messages = vec![
        did_open(&uri, "fn main() {\n    print_str(\"hi\");\n}\n"),
        request(2, "textDocument/hover", at(&uri, 1, 6)),
    ];
    let replies = run(messages);
    let hover = replies
        .iter()
        .find(|reply| reply.get("id").and_then(Value::as_i64) == Some(2))
        .expect("the hover request gets a reply");
    let value = hover
        .get_path(&["result", "contents", "value"])
        .and_then(Value::as_str)
        .expect("the hover carries markup");
    assert!(value.contains("print_str"), "the name is shown: {value}");
    assert!(value.contains("string"), "the parameter type is shown: {value}");
}

#[test]
fn definition_jumps_to_a_function_in_another_file() {
    let dir = std::env::temp_dir().join(format!("yashy-lsp-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("the test directory must be creatable");
    let helper = dir.join("helper.ya");
    std::fs::write(
        &helper,
        "fn double(x: i64) -> i64 {\n    return x * 2;\n}\n",
    )
    .expect("the helper module must be writable");
    let main_path = dir.join("definition_main.ya");
    let main_text = "import helper;\n\nfn main() {\n    print_i64(double(21));\n}\n";
    std::fs::write(&main_path, main_text).expect("the entry module must be writable");

    let uri = path_to_uri(&main_path);
    let messages = vec![
        did_open(&uri, main_text),
        request(2, "textDocument/definition", at(&uri, 3, 16)),
    ];
    let replies = run(messages);
    let definition = replies
        .iter()
        .find(|reply| reply.get("id").and_then(Value::as_i64) == Some(2))
        .expect("the definition request gets a reply");
    let jump = definition
        .get("result")
        .expect("the definition has a result");
    let target = jump
        .get("uri")
        .and_then(Value::as_str)
        .expect("the location has a uri");
    assert_eq!(
        yashy_lsp::uri_to_path(target),
        helper,
        "the jump lands in the imported module"
    );
    let line = jump
        .get_path(&["range", "start", "line"])
        .and_then(Value::as_i64)
        .expect("the location has a range");
    assert_eq!(line, 0, "`double` is declared on the first line");
}

#[test]
fn semantic_tokens_cover_keywords_types_and_functions() {
    let uri = test_uri("tokens.ya");
    let text = "// a comment\nfn add(a: i64) -> i64 {\n    return a + 1;\n}\nfn main() {\n    print_str(\"hi\");\n}\n";
    let messages = vec![
        request(1, "initialize", object([("capabilities", object([]))])),
        did_open(&uri, text),
        request(2, "textDocument/semanticTokens/full", object([("textDocument", object([("uri", string(&uri))]))])),
    ];
    let replies = run(messages);
    let initialize = replies
        .iter()
        .find(|reply| reply.get("id").and_then(Value::as_i64) == Some(1))
        .expect("the initialize reply");
    let legend = initialize
        .get_path(&["result", "capabilities", "semanticTokensProvider", "legend", "tokenTypes"])
        .and_then(Value::as_array)
        .expect("the legend is advertised");
    assert_eq!(legend.len(), 6, "six token types");

    let tokens = replies
        .iter()
        .find(|reply| reply.get("id").and_then(Value::as_i64) == Some(2))
        .expect("the semantic-token reply");
    let data = tokens
        .get_path(&["result", "data"])
        .and_then(Value::as_array)
        .expect("the reply carries token data");
    assert_eq!(data.len() % 5, 0, "tokens are five integers each");
    let used: std::collections::BTreeSet<i64> = data
        .chunks(5)
        .filter_map(|token| token.get(3).and_then(Value::as_i64))
        .collect();
    for (index, name) in ["keyword", "type", "function", "string", "number", "comment"]
        .iter()
        .enumerate()
    {
        assert!(
            used.contains(&(index as i64)),
            "no `{name}` token was produced: {used:?}"
        );
    }
}

#[test]
fn the_server_survives_a_file_that_does_not_parse() {
    let uri = test_uri("half-typed.ya");
    let messages = vec![
        did_open(&uri, "fn main( {\n"),
        request(2, "textDocument/hover", at(&uri, 0, 0)),
    ];
    let replies = run(messages);
    assert!(
        publish(&replies).is_some(),
        "half-typed input still gets diagnostics"
    );
    let hover = replies
        .iter()
        .find(|reply| reply.get("id").and_then(Value::as_i64) == Some(2))
        .expect("the server answered after a parse error");
    assert!(
        hover.get("result").is_some(),
        "the hover reply has a result field, even when it is null"
    );
}

#[test]
fn a_uri_keeps_a_windows_paths_characters() {
    let path = PathBuf::from(r"D:\Yashy\examples\main.ya");
    let decoded = yashy_lsp::uri_to_path(&path_to_uri(&path));
    assert_eq!(
        decoded.to_string_lossy().replace('\\', "/"),
        "D:/Yashy/examples/main.ya"
    );
}

#[test]
fn a_native_path_round_trips_through_a_uri() {
    let path = std::env::temp_dir()
        .join("yashy lsp")
        .join("examples")
        .join("main.ya");
    assert_eq!(yashy_lsp::uri_to_path(&path_to_uri(&path)), path);
}
