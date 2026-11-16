//! A half-typed file must not take the language server down.
//!
//! The editor keeps asking for hover and completion while the user is still
//! typing. Recovery in the parser means those requests still get an answer and
//! the diagnostic still has a range, even when the buffer does not parse.

use std::io::Cursor;

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

fn did_open(uri: &str, text: &str) -> Value {
    object([
        ("jsonrpc", string("2.0")),
        ("method", string("textDocument/didOpen")),
        (
            "params",
            object([(
                "textDocument",
                object([
                    ("uri", string(uri)),
                    ("languageId", string("yashy")),
                    ("version", number(1)),
                    ("text", string(text)),
                ]),
            )]),
        ),
    ])
}

fn at(uri: &str, line: i64, character: i64) -> Value {
    object([
        ("textDocument", object([("uri", string(uri))])),
        (
            "position",
            object([("line", number(line)), ("character", number(character))]),
        ),
    ])
}

fn run(messages: Vec<Value>) -> Vec<Value> {
    let mut input = Vec::new();
    for message in messages {
        let body = to_string(&message);
        use std::io::Write as _;
        write!(input, "Content-Length: {}\r\n\r\n{body}", body.len()).unwrap();
    }
    let mut output = Vec::new();
    yashy_lsp::serve(Cursor::new(input), &mut output).expect("the server must survive");
    let text = String::from_utf8(output).expect("UTF-8 replies");
    let mut replies = Vec::new();
    let mut rest = text.as_str();
    while let Some(end) = rest.find("\r\n\r\n") {
        let length: usize = rest[..end]
            .strip_prefix("Content-Length: ")
            .and_then(|value| value.trim().parse().ok())
            .expect("every reply has a length");
        let start = end + 4;
        let body = &rest[start..start + length];
        replies.push(yashy_lsp::json::parse(body).expect("JSON reply"));
        rest = &rest[start + length..];
    }
    replies
}

#[test]
fn hover_and_completion_answer_on_a_broken_buffer() {
    let uri = path_to_uri(
        &std::env::temp_dir().join(format!("yashy-lsp-recovery-{}.ya", std::process::id())),
    );
    let replies = run(vec![
        did_open(&uri, "fn main() {\n    const int x = ;\n    pri\n"),
        request(1, "textDocument/hover", at(&uri, 1, 10)),
        request(2, "textDocument/completion", at(&uri, 2, 6)),
    ]);

    let diagnostic = replies
        .iter()
        .find(|reply| {
            reply.get("method").and_then(Value::as_str) == Some("textDocument/publishDiagnostics")
        })
        .expect("a diagnostic must be published for the broken buffer");
    let list = diagnostic
        .get_path(&["params", "diagnostics"])
        .and_then(Value::as_array)
        .expect("the diagnostic list");
    assert!(
        !list.is_empty(),
        "the half-typed declaration must be reported"
    );
    assert!(
        list.first().and_then(|d| d.get("range")).is_some(),
        "the diagnostic must carry a range"
    );

    for id in [1, 2] {
        assert!(
            replies
                .iter()
                .any(|reply| reply.get("id").and_then(Value::as_i64) == Some(id)),
            "request {id} must get a reply even when the buffer does not parse"
        );
    }
}

#[test]
fn a_recovery_only_buffer_never_reports_success_to_the_editor() {
    let uri = path_to_uri(&std::env::temp_dir().join(format!(
        "yashy-lsp-recovery-clean-{}.ya",
        std::process::id()
    )));
    let replies = run(vec![did_open(&uri, "fn main(\n")]);
    let published = replies
        .iter()
        .find(|reply| {
            reply.get("method").and_then(Value::as_str) == Some("textDocument/publishDiagnostics")
        })
        .expect("a broken signature must still publish diagnostics");
    let list = published
        .get_path(&["params", "diagnostics"])
        .and_then(Value::as_array)
        .expect("the diagnostic list");
    assert!(
        !list.is_empty(),
        "an unclosed signature is not a clean file"
    );
}
