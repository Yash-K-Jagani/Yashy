//! The editor definitions and the lexer must agree on the keyword list.
//!
//! This lives in `otter-lsp` because it needs a JSON parser and this crate
//! already has one; a dev-dependency from the lexer on the LSP would be a
//! cycle.

use std::path::PathBuf;

use otter_lsp::json::Value;

fn editors() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../editors")
}

fn read(path: PathBuf) -> String {
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{} must be readable: {error}", path.display()))
}

/// Every word in a `match` of the form `\b(?:a|b|c)\b`.
fn alternatives(pattern: &str) -> Vec<String> {
    let Some(open) = pattern.find("(?:") else {
        return Vec::new();
    };
    let rest = &pattern[open + 3..];
    let Some(close) = rest.find(')') else {
        return Vec::new();
    };
    rest[..close]
        .split('|')
        .map(str::trim)
        .filter(|word| !word.is_empty())
        .map(str::to_string)
        .collect()
}

fn collect_keywords(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::Obj(fields) => {
            if let Some(name) = value.get("name").and_then(Value::as_str) {
                if name.starts_with("keyword.") {
                    if let Some(pattern) = value.get("match").and_then(Value::as_str) {
                        out.extend(alternatives(pattern));
                    }
                }
            }
            for (_, child) in fields {
                collect_keywords(child, out);
            }
        }
        Value::Arr(items) => {
            for item in items {
                collect_keywords(item, out);
            }
        }
        _ => {}
    }
}

#[test]
fn the_grammar_lists_exactly_the_lexer_keywords() {
    let path = editors().join("vscode/syntaxes/otter.tmLanguage.json");
    let grammar = otter_lsp::json::parse(&read(path)).expect("the grammar must be JSON");
    let mut actual = Vec::new();
    collect_keywords(&grammar, &mut actual);
    actual.sort();
    actual.dedup();

    let mut expected: Vec<String> = otter_lexer::TokenKind::KEYWORDS
        .iter()
        .map(|keyword| (*keyword).to_string())
        .collect();
    expected.sort();

    assert_eq!(
        actual, expected,
        "the TextMate grammar and `TokenKind::KEYWORDS` disagree"
    );
}

#[test]
fn every_editor_definition_lists_every_keyword() {
    let vim = read(editors().join("vim/otter.vim"));
    let emacs = read(editors().join("emacs/otter.el"));
    for keyword in otter_lexer::TokenKind::KEYWORDS {
        assert!(
            vim.split_whitespace().any(|word| word == keyword),
            "`editors/vim/otter.vim` is missing `{keyword}`"
        );
        assert!(
            emacs.contains(&format!("\"{keyword}\"")),
            "`editors/emacs/otter.el` is missing `{keyword}`"
        );
    }
}

#[test]
fn the_vscode_extension_points_at_its_grammar_and_configuration() {
    let manifest = read(editors().join("vscode/package.json"));
    let manifest = otter_lsp::json::parse(&manifest).expect("the manifest must be JSON");
    let grammar = manifest
        .get_path(&["contributes", "grammars"])
        .and_then(Value::as_array)
        .and_then(|grammars| grammars.first())
        .expect("the manifest contributes a grammar");
    let path = grammar
        .get("path")
        .and_then(Value::as_str)
        .expect("the grammar has a path");
    let scope = grammar
        .get("scopeName")
        .and_then(Value::as_str)
        .expect("the grammar has a scope");
    assert_eq!(scope, "source.otter");
    assert!(
        editors().join("vscode").join(path.trim_start_matches("./")).is_file(),
        "the grammar path `{path}` must exist"
    );
    let configuration = read(editors().join("vscode/language-configuration.json"));
    otter_lsp::json::parse(&configuration).expect("the language configuration must be JSON");
}
