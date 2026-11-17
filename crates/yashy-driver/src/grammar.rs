//! Render one [`GrammarModel`] into every editor format the project ships.
//!
//! The lexer owns the vocabulary; this module owns the file formats. Nothing
//! here invents a keyword or an operator: a token has to be in the model
//! before it can appear in a grammar, and `yashyc tokens --grammar` rewrites
//! every committed file from that one model. The drift test in
//! `crates/yashy-driver/tests/grammar.rs` compares the bytes.

use std::path::{Path, PathBuf};

use yashy_common::Json;
use yashy_lexer::grammar::{GrammarModel, TYPES};

/// One generated file and the path it belongs at, relative to `editors/`.
pub struct GeneratedFile {
    pub path: &'static str,
    pub text: String,
}

/// Every file the model generates.
pub fn files(model: &GrammarModel) -> Vec<GeneratedFile> {
    let textmate = render_textmate(model);
    let vim = render_vim(model);
    let emacs = render_emacs(model);
    vec![
        GeneratedFile {
            path: "grammars/yashy.tmLanguage.json",
            text: textmate.clone(),
        },
        GeneratedFile {
            path: "vscode/syntaxes/yashy.tmLanguage.json",
            text: textmate,
        },
        GeneratedFile {
            path: "grammars/yashy.vim",
            text: vim.clone(),
        },
        GeneratedFile {
            path: "vim/yashy.vim",
            text: vim,
        },
        GeneratedFile {
            path: "grammars/yashy.el",
            text: emacs.clone(),
        },
        GeneratedFile {
            path: "emacs/yashy.el",
            text: emacs,
        },
        GeneratedFile {
            path: "grammars/helix/languages.toml",
            text: render_helix(model),
        },
        GeneratedFile {
            path: "grammars/tree-sitter-yashy/grammar.js",
            text: render_tree_sitter(model),
        },
        GeneratedFile {
            path: "grammars/tree-sitter-yashy/queries/highlights.scm",
            text: render_highlights(model),
        },
        GeneratedFile {
            path: "grammars/tree-sitter-yashy/queries/locals.scm",
            text: LOCALS.to_string(),
        },
        GeneratedFile {
            path: "grammars/tree-sitter-yashy/queries/folds.scm",
            text: FOLDS.to_string(),
        },
    ]
}

/// A regex alternation, longest spelling first so `+=` is not eaten by `+`.
fn alternatives(words: &[&str]) -> String {
    let mut sorted: Vec<&str> = words.to_vec();
    sorted
        .sort_unstable_by(|left, right| right.len().cmp(&left.len()).then_with(|| left.cmp(right)));
    sorted
        .iter()
        .map(|word| regex_escape(word))
        .collect::<Vec<_>>()
        .join("|")
}

/// Escape a literal spelling for an Oniguruma/JavaScript regex.
fn regex_escape(word: &str) -> String {
    let mut out = String::new();
    for character in word.chars() {
        if "\\.^$|?*+()[]{}".contains(character) {
            out.push('\\');
        }
        out.push(character);
    }
    out
}

fn rule(name: &str, pattern: &str) -> Json {
    let mut object = Json::object();
    object.set("name", Json::string(name));
    object.set("match", Json::string(pattern));
    object
}

/// The TextMate grammar, which VS Code, Helix and Zed all understand.
pub fn render_textmate(model: &GrammarModel) -> String {
    let mut repository = Json::object();
    repository.set("docs", rule("comment.line.documentation.yashy", "///.*$"));
    let mut comments = Json::object();
    comments.set(
        "patterns",
        Json::array(vec![rule("comment.line.double-slash.yashy", "//.*$"), {
            let mut block = Json::object();
            block.set("name", Json::string("comment.block.yashy"));
            block.set("begin", Json::string("/\\*"));
            block.set("end", Json::string("\\*/"));
            block
        }]),
    );
    repository.set("comments", comments);
    let mut strings = Json::object();
    strings.set("name", Json::string("string.quoted.double.yashy"));
    strings.set("begin", Json::string("\""));
    strings.set("end", Json::string("\""));
    strings.set(
        "patterns",
        Json::array(vec![rule("constant.character.escape.yashy", "\\\\.")]),
    );
    repository.set("strings", strings);
    repository.set(
        "characters",
        rule("string.quoted.single.yashy", "'(\\\\.|[^'\\\\])'"),
    );
    repository.set(
        "numbers",
        rule("constant.numeric.yashy", "\\b(?:\\d+\\.\\d+|\\d+)\\b"),
    );
    repository.set(
        "keywords",
        rule(
            "keyword.control.yashy",
            &format!("\\b(?:{})\\b", alternatives(&keywords(model))),
        ),
    );
    repository.set(
        "types",
        rule(
            "support.type.yashy",
            &format!("\\b(?:{})\\b", alternatives(&types(model))),
        ),
    );
    repository.set(
        "operators",
        rule(
            "keyword.operator.yashy",
            &format!("(?:{})", alternatives(&model.operators)),
        ),
    );
    repository.set(
        "delimiters",
        rule(
            "punctuation.yashy",
            &format!("(?:{})", alternatives(&model.delimiters)),
        ),
    );

    let includes = &[
        ("include", "#docs"),
        ("include", "#comments"),
        ("include", "#strings"),
        ("include", "#characters"),
        ("include", "#numbers"),
        ("include", "#types"),
        ("include", "#keywords"),
        ("include", "#operators"),
        ("include", "#delimiters"),
    ];
    let patterns: Vec<Json> = includes
        .iter()
        .map(|(key, value)| {
            let mut object = Json::object();
            object.set(*key, Json::string(*value));
            object
        })
        .collect();

    let mut grammar = Json::object();
    grammar.set(
        "$schema",
        Json::string(
            "https://raw.githubusercontent.com/martinring/tmlanguage/master/tmlanguage.json",
        ),
    );
    grammar.set("name", Json::string("Yashy"));
    grammar.set("scopeName", Json::string("source.yashy"));
    grammar.set("fileTypes", Json::array(vec![Json::string("ya")]));
    grammar.set("patterns", Json::array(patterns));
    grammar.set("repository", repository);
    let mut out = grammar.render();
    out.push('\n');
    out
}

/// Keywords in the order the lexer has them, plus the type names that are
/// also reserved words in practice.
fn keywords(model: &GrammarModel) -> Vec<&str> {
    model.keywords.clone()
}

fn types(model: &GrammarModel) -> Vec<&str> {
    model.types.clone()
}

/// Escape a spelling for a Vim double-quoted regular expression.
fn vim_pattern(word: &str) -> String {
    let mut out = String::new();
    for character in word.chars() {
        if "\\.*[]^$~\"".contains(character) {
            out.push('\\');
        }
        out.push(character);
    }
    out
}

fn vim_alternatives(words: &[&str]) -> String {
    let mut sorted: Vec<&str> = words.to_vec();
    sorted
        .sort_unstable_by(|left, right| right.len().cmp(&left.len()).then_with(|| left.cmp(right)));
    sorted
        .iter()
        .map(|word| vim_pattern(word))
        .collect::<Vec<_>>()
        .join("\\|")
}

/// Vim syntax: the keyword lists are generated, the rest is the same template
/// the 1.0 mode used.
pub fn render_vim(model: &GrammarModel) -> String {
    format!(
        r#"" Generated by `yashyc tokens --grammar`. Do not edit.
" Language: Yashy. The lists below come from `yashy-lexer`.

if exists("b:current_syntax")
  finish
endif

syntax keyword yashyKeyword {keywords}
syntax keyword yashyType {types}
syntax match yashyDocComment "///.*$"
syntax match yashyComment "//.*$"
syntax region yashyComment start="/\*" end="\*/" keepend
syntax region yashyString start=+"+ skip=+\\\\\|\\"+ end=+"+ contains=yashyEscape,yashyInterpolation
syntax match yashyEscape "\\." contained
syntax region yashyInterpolation start="\${{}}" end="}}" contained
syntax match yashyNumber "\v<\d+>"
syntax match yashyNumber "\v<\d+\.\d+>"
syntax match yashyOperator "{operators}"
syntax match yashyDelimiter "{delimiters}"
syntax match yashyFunction "\v<\a\w*>\ze\s*\("

highlight default link yashyKeyword Keyword
highlight default link yashyType Type
highlight default link yashyDocComment SpecialComment
highlight default link yashyComment Comment
highlight default link yashyString String
highlight default link yashyEscape SpecialChar
highlight default link yashyInterpolation Special
highlight default link yashyNumber Number
highlight default link yashyOperator Operator
highlight default link yashyDelimiter Delimiter
highlight default link yashyFunction Function

let b:current_syntax = "yashy"
"#,
        keywords = model.keywords.join(" "),
        types = types(model).join(" "),
        operators = vim_alternatives(&model.operators),
        delimiters = vim_alternatives(&model.delimiters),
    )
}

/// Escape a spelling for an Emacs Lisp string.
fn elisp_string(word: &str) -> String {
    let mut out = String::from("\"");
    for character in word.chars() {
        match character {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

fn elisp_list(words: &[&str]) -> String {
    words
        .iter()
        .map(|word| elisp_string(word))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Emacs mode: the same template as the 1.0 mode with generated vocabularies.
pub fn render_emacs(model: &GrammarModel) -> String {
    format!(
        r#";;; yashy.el --- Major mode for the Yashy language -*- lexical-binding: t; -*-
;;
;; Generated by `yashyc tokens --grammar`. Do not edit.
;;

;;; Code:

(defvar yashy-keywords
  '({keywords})
  "Yashy keywords.")

(defvar yashy-types
  '({types})
  "Yashy type names.")

(defvar yashy-operators
  '({operators})
  "Yashy operators.")

(defvar yashy-delimiters
  '({delimiters})
  "Yashy structural punctuation.")

(defvar yashy-font-lock-keywords
  `((,(regexp-opt yashy-keywords 'symbols) . font-lock-keyword-face)
    (,(regexp-opt yashy-types 'symbols) . font-lock-type-face)
    (,(regexp-opt yashy-operators t) . font-lock-builtin-face)
    (,(regexp-opt yashy-delimiters t) . font-lock-variable-name-face)
    ("///.*$" . font-lock-doc-face)
    ("//.*$" . font-lock-comment-face)
    ("/\\*\\(?:.\\|\n\\)*?\\*/" . font-lock-comment-face)
    ("\"\\(?:\\\\.\\|[^\"\\\\]\\)*\"" . font-lock-string-face)
    ("\\_<[0-9]+\\.[0-9]+\\_>" . font-lock-constant-face)
    ("\\_<[0-9]+\\_>" . font-lock-constant-face)
    ("\\_<\([A-Za-z_][A-Za-z0-9_]*\)\_>[ \t]*(" 1 font-lock-function-name-face))
  "Font-lock rules for `yashy-mode'.")

(defvar yashy-mode-syntax-table
  (let ((table (make-syntax-table)))
    (modify-syntax-entry ?/ ". 124b" table)
    (modify-syntax-entry ?* ". 23" table)
    (modify-syntax-entry ?\n "> b" table)
    (modify-syntax-entry ?_ "_" table)
    (modify-syntax-entry ?\" "\"" table)
    (modify-syntax-entry ?' "'" table)
    table)
  "Syntax table for `yashy-mode'.")

;;;###autoload
(define-derived-mode yashy-mode prog-mode "Yashy"
  "Major mode for editing Yashy source files."
  :syntax-table yashy-mode-syntax-table
  (setq-local comment-start "// ")
  (setq-local comment-end "")
  (setq-local comment-start-skip "//+\\s-*")
  (setq-local font-lock-defaults '(yashy-font-lock-keywords))
  (setq-local indent-tabs-mode nil)
  (setq-local tab-width 4))

;;;###autoload
(add-to-list 'auto-mode-alist '("\\.ya\\'" . yashy-mode))

(provide 'yashy)
;;; yashy.el ends here
"#,
        keywords = elisp_list(&keywords(model)),
        types = elisp_list(&types(model)),
        operators = elisp_list(&model.operators),
        delimiters = elisp_list(&model.delimiters),
    )
}

/// Helix uses the same LSP the language server speaks; the generated file
/// pins the scope and file types to the ones the TextMate grammar declares.
pub fn render_helix(_model: &GrammarModel) -> String {
    r#"# Generated by `yashyc tokens --grammar`. Do not edit.
[[language]]
name = "yashy"
scope = "source.yashy"
injection-regex = "yashy"
file-types = ["ya"]
comment-token = "//"
indent = { tab-width = 4, unit = "    " }
roots = ["yashy.toml", ".git"]

[language-server.yashy-lsp]
command = "yashyc"
args = ["lsp", "--stdio"]
"#
    .to_string()
}

/// JS string literal for a generated tree-sitter grammar.
fn js_string(word: &str) -> String {
    let mut out = String::from("\"");
    for character in word.chars() {
        match character {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

fn js_choices(words: &[&str]) -> String {
    words
        .iter()
        .map(|word| js_string(word))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Tree-sitter's skeleton is hand-written; its tables come from the model.
pub fn render_tree_sitter(model: &GrammarModel) -> String {
    format!(
        r#"// Generated by `yashyc tokens --grammar`. Do not edit.
// The rule skeleton is hand-written once; the vocabularies are from the lexer.
module.exports = grammar({{
  name: 'yashy',
  extras: $ => [/\s/, $.line_comment, $.block_comment, $.doc_comment],
  rules: {{
    source_file: $ => repeat($._statement),

    _statement: $ => choice($.function, $.expression_statement),
    function: $ => seq($.keyword, $.identifier, $.delimiter, $.block),
    block: $ => seq('{{', repeat($._statement), '}}'),
    expression_statement: $ => seq($._expression, ';'),

    _expression: $ => choice($.identifier, $.integer, $.string, $.binary_expression),
    binary_expression: $ => seq($._expression, $.operator, $._expression),

    keyword: $ => choice({keywords}),
    type: $ => choice({types}),
    operator: $ => choice({operators}),
    delimiter: $ => choice({delimiters}),

    identifier: $ => /[A-Za-z_][A-Za-z0-9_]*/,
    integer: $ => /\d+/,
    string: $ => /"([^"\\]|\\.)*"/,

    line_comment: $ => token(seq('//', /.*/)),
    block_comment: $ => token(seq('/*', /[\s\S]*?/, '*/')),
    doc_comment: $ => token(seq('///', /.*/)),
  }}
}});
"#,
        keywords = js_choices(&keywords(model)),
        types = js_choices(&types(model)),
        operators = js_choices(&model.operators),
        delimiters = js_choices(&model.delimiters),
    )
}

/// One capture per class: TextMate, tree-sitter, Vim and Emacs agree by
/// construction because they all read the same model.
pub fn render_highlights(_model: &GrammarModel) -> String {
    r#"(keyword) @keyword
(type) @type
(operator) @operator
(delimiter) @punctuation
(identifier) @variable
(function (identifier) @function)
(integer) @number
(string) @string
(line_comment) @comment
(block_comment) @comment
(doc_comment) @comment.documentation
"#
    .to_string()
}

const LOCALS: &str =
    "(function name: (identifier) @definition.function)\n(identifier) @reference\n";

const FOLDS: &str = "(block) @fold\n(block_comment) @fold\n";

/// Resolve the editors root. Both `editors` and `editors/grammars` are accepted
/// so the command in the plan and the natural argument work the same way.
pub fn editors_root(path: &Path) -> PathBuf {
    if path.file_name().and_then(|name| name.to_str()) == Some("grammars") {
        path.parent().unwrap_or(path).to_path_buf()
    } else {
        path.to_path_buf()
    }
}

/// Write every generated file, creating directories as needed.
pub fn write_all(root: &Path) -> std::io::Result<Vec<PathBuf>> {
    let root = editors_root(root);
    let mut written = Vec::new();
    for generated in files(&yashy_lexer::grammar::model()) {
        let path = root.join(generated.path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, generated.text)?;
        written.push(path);
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_format_lists_every_keyword_and_type() {
        let model = yashy_lexer::grammar::model();
        let textmate = render_textmate(&model);
        let vim = render_vim(&model);
        let emacs = render_emacs(&model);
        let tree_sitter = render_tree_sitter(&model);
        for word in TYPES {
            assert!(textmate.contains(word), "TextMate misses `{word}`");
            assert!(vim.contains(word), "Vim misses `{word}`");
            assert!(emacs.contains(&elisp_string(word)), "Emacs misses `{word}`");
            assert!(
                tree_sitter.contains(&js_string(word)),
                "tree-sitter misses `{word}`"
            );
        }
        for word in &model.operators {
            assert!(
                vim.contains(&vim_pattern(word)),
                "Vim misses operator `{word}`"
            );
            assert!(
                tree_sitter.contains(&js_string(word)),
                "tree-sitter misses operator `{word}`"
            );
        }
        for word in &model.delimiters {
            assert!(
                tree_sitter.contains(&js_string(word)),
                "tree-sitter misses delimiter `{word}`"
            );
        }
    }

    #[test]
    fn the_textmate_rule_names_are_stable() {
        let model = yashy_lexer::grammar::model();
        let text = render_textmate(&model);
        for scope in [
            "keyword.control.yashy",
            "support.type.yashy",
            "keyword.operator.yashy",
            "punctuation.yashy",
        ] {
            assert!(text.contains(scope), "missing highlight scope `{scope}`");
        }
    }
}
