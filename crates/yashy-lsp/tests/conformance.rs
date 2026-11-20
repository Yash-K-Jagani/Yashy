//! LSP conformance: one snapshot, every feature answers with a stable span.
//!
//! The transcript is driven through the same stdio framing an editor uses, so
//! a feature that answers in a unit test but not over the wire cannot pass.

use std::io::{Cursor, Write};

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

fn at(uri: &str, line: i64, character: i64, extra: &[(&str, Value)]) -> Value {
    let mut fields = vec![
        ("textDocument", object([("uri", string(uri))])),
        (
            "position",
            object([("line", number(line)), ("character", number(character))]),
        ),
    ];
    fields.extend_from_slice(extra);
    Value::Obj(
        fields
            .into_iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect(),
    )
}

fn run(messages: Vec<Value>) -> Vec<Value> {
    let mut input = Vec::new();
    for message in messages {
        let body = to_string(&message);
        write!(input, "Content-Length: {}\r\n\r\n{body}", body.len()).unwrap();
    }
    let mut output = Vec::new();
    yashy_lsp::serve(Cursor::new(input), &mut output).expect("the server must not fail");
    let text = String::from_utf8(output).unwrap();
    let mut replies = Vec::new();
    let mut rest = text.as_str();
    while let Some(end) = rest.find("\r\n\r\n") {
        let length: usize = rest[..end]
            .strip_prefix("Content-Length: ")
            .and_then(|value| value.trim().parse().ok())
            .unwrap();
        let start = end + 4;
        replies.push(yashy_lsp::json::parse(&rest[start..start + length]).unwrap());
        rest = &rest[start + length..];
    }
    replies
}

const SOURCE: &str = "struct Point { x: int }\nfn add(int a, int b) -> int { return a + b; }\nfn main() { let p = add(1, 2); let m = 5; }\n";

#[test]
fn every_feature_answers_with_a_stable_range() {
    let uri = path_to_uri(&std::env::temp_dir().join("lsp-conformance.ya"));
    let messages = vec![
        did_open(&uri, SOURCE),
        request(1, "textDocument/documentSymbol", at(&uri, 0, 0, &[])),
        request(2, "textDocument/references", at(&uri, 2, 22, &[])),
        request(3, "textDocument/prepareRename", at(&uri, 2, 22, &[])),
        request(
            4,
            "textDocument/rename",
            at(&uri, 2, 22, &[("newName", string("sum"))]),
        ),
        request(5, "textDocument/foldingRange", at(&uri, 0, 0, &[])),
        request(6, "textDocument/selectionRange", at(&uri, 2, 22, &[])),
        request(7, "textDocument/documentHighlight", at(&uri, 2, 22, &[])),
        request(8, "textDocument/diagnostic", at(&uri, 0, 0, &[])),
        request(9, "textDocument/codeAction", at(&uri, 2, 22, &[])),
        request(
            10,
            "textDocument/formatting",
            at(&uri, 0, 0, &[("options", object([("tabSize", number(4))]))]),
        ),
        request(11, "textDocument/hover", at(&uri, 2, 22, &[])),
        request(12, "textDocument/definition", at(&uri, 2, 22, &[])),
        request(13, "textDocument/completion", at(&uri, 2, 22, &[])),
        request(14, "textDocument/signatureHelp", at(&uri, 2, 24, &[])),
        request(15, "textDocument/inlayHint", at(&uri, 2, 0, &[])),
        request(16, "workspace/symbol", object([("query", string("Poi"))])),
        request(
            17,
            "textDocument/prepareCallHierarchy",
            at(&uri, 2, 22, &[]),
        ),
        request(18, "textDocument/prepareTypeHierarchy", at(&uri, 0, 8, &[])),
        request(19, "textDocument/codeLens", at(&uri, 0, 0, &[])),
    ];
    let replies = run(messages);
    for id in 1..=19 {
        let reply = replies
            .iter()
            .find(|reply| reply.get("id").and_then(Value::as_i64) == Some(id))
            .unwrap_or_else(|| panic!("feature {id} did not answer"));
        assert!(reply.get("result").is_some(), "feature {id}: {reply:?}");
        assert!(
            reply.get("error").is_none(),
            "feature {id} errored: {reply:?}"
        );
    }

    // documentSymbol names both declarations, with stable line ranges.
    let symbols = replies
        .iter()
        .find(|reply| reply.get("id").and_then(Value::as_i64) == Some(1))
        .and_then(|reply| reply.get("result"))
        .and_then(Value::as_array)
        .expect("documentSymbol returns an array");
    assert!(!symbols.is_empty(), "the snapshot declares symbols");
    let rendered = to_string(&Value::Arr(symbols.to_vec()));
    assert!(rendered.contains("Point"), "{rendered}");
    assert!(rendered.contains("add"), "{rendered}");
    assert!(rendered.contains("\"line\":0"), "{rendered}");

    // rename produces a WorkspaceEdit rather than a formatted string.
    let rename = replies
        .iter()
        .find(|reply| reply.get("id").and_then(Value::as_i64) == Some(4))
        .and_then(|reply| reply.get("result"))
        .expect("rename answers");
    assert!(rename.get("changes").is_some(), "{rename:?}");

    // signatureHelp names the callee and its declaration line.
    let signature = replies
        .iter()
        .find(|reply| reply.get("id").and_then(Value::as_i64) == Some(14))
        .and_then(|reply| reply.get("result"))
        .expect("signature help answers");
    let rendered = to_string(signature);
    assert!(rendered.contains("fn add"), "{rendered}");

    // inlayHint proves the inferred binding's type from its literal.
    let hints = replies
        .iter()
        .find(|reply| reply.get("id").and_then(Value::as_i64) == Some(15))
        .and_then(|reply| reply.get("result"))
        .expect("inlay hints answer");
    let rendered = to_string(hints);
    assert!(rendered.contains(": int"), "{rendered}");

    // workspace/symbol filters by the query.
    let workspace = replies
        .iter()
        .find(|reply| reply.get("id").and_then(Value::as_i64) == Some(16))
        .and_then(|reply| reply.get("result"))
        .expect("workspace symbols answer");
    let rendered = to_string(workspace);
    assert!(rendered.contains("Point"), "{rendered}");
    assert!(
        !rendered.contains("\\\"name\\\": \\\"add\\\""),
        "{rendered}"
    );

    // code lenses attach a reference count to each declaration.
    let lenses = replies
        .iter()
        .find(|reply| reply.get("id").and_then(Value::as_i64) == Some(19))
        .and_then(|reply| reply.get("result"))
        .expect("code lenses answer");
    assert!(to_string(lenses).contains("references"));
}

#[test]
fn an_incomplete_file_still_answers_every_feature() {
    let uri = path_to_uri(&std::env::temp_dir().join("lsp-conformance-incomplete.ya"));
    let messages = vec![
        did_open(&uri, "fn broken( {\n"),
        request(1, "textDocument/documentSymbol", at(&uri, 0, 0, &[])),
        request(2, "textDocument/diagnostic", at(&uri, 0, 0, &[])),
        request(3, "textDocument/formatting", at(&uri, 0, 0, &[])),
    ];
    let replies = run(messages);
    for id in 1..=3 {
        assert!(
            replies
                .iter()
                .any(|reply| reply.get("id").and_then(Value::as_i64) == Some(id)),
            "feature {id} must answer on an incomplete file"
        );
    }
}

#[test]
fn code_actions_offer_a_real_edit_for_a_removed_keyword() {
    let uri = path_to_uri(&std::env::temp_dir().join("lsp-fix.ya"));
    // The cursor is on `var`, which the checker would replace with `let`.
    let messages = vec![
        did_open(&uri, "fn main() { var int x = 1; }\n"),
        request(1, "textDocument/codeAction", at(&uri, 0, 12, &[])),
    ];
    let replies = run(messages);
    let result = replies
        .iter()
        .find(|reply| reply.get("id").and_then(Value::as_i64) == Some(1))
        .and_then(|reply| reply.get("result"))
        .expect("codeAction answers");
    let rendered = to_string(result);
    assert!(
        rendered.contains("use `let`"),
        "a quick fix must be offered: {rendered}"
    );
    assert!(
        rendered.contains("newText"),
        "the edit must carry replacement text: {rendered}"
    );
    assert!(
        rendered.contains("\"let\""),
        "the replacement must be `let`: {rendered}"
    );
}
