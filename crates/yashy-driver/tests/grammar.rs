//! The committed editor grammars are generated, never hand-edited.
//!
//! `yashyc tokens --grammar editors` rewrites every file checked here. A new
//! keyword or operator changes the model, which changes the rendered bytes,
//! which fails this test until the files are regenerated and committed.

use std::path::Path;

use yashy_driver::grammar;
use yashy_lexer::grammar as vocabulary;

fn editors() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../editors")
}

/// The TextMate and tree-sitter renderers escape regex metacharacters in a
/// literal spelling (`..` becomes `\.\.`), so a vocabulary word may appear in
/// either form. The check is about presence, not about which renderer escapes.
fn escaped(word: &str) -> String {
    let mut out = String::new();
    for character in word.chars() {
        if "\\.^$|?*+()[]{}".contains(character) {
            out.push('\\');
        }
        out.push(character);
    }
    out
}

#[test]
fn every_committed_grammar_is_the_rendered_one() {
    let model = vocabulary::model();
    let generated = grammar::files(&model);
    assert_eq!(
        generated.len(),
        11,
        "the generator must own every editor file"
    );
    for file in generated {
        let path = editors().join(file.path);
        let committed = std::fs::read(&path).unwrap_or_else(|error| {
            panic!(
                "{} must exist; run `yashyc tokens --grammar editors`: {error}",
                path.display()
            )
        });
        assert_eq!(
            committed,
            file.text.as_bytes(),
            "{} is stale; run `yashyc tokens --grammar editors`",
            file.path
        );
    }
}

#[test]
fn the_copies_are_identical_to_the_canonical_generated_files() {
    let model = vocabulary::model();
    let textmate = grammar::render_textmate(&model);
    let vim = grammar::render_vim(&model);
    let emacs = grammar::render_emacs(&model);
    for (canonical, copy) in [
        (
            "grammars/yashy.tmLanguage.json",
            "vscode/syntaxes/yashy.tmLanguage.json",
        ),
        ("grammars/yashy.vim", "vim/yashy.vim"),
        ("grammars/yashy.el", "emacs/yashy.el"),
    ] {
        let canonical = std::fs::read(editors().join(canonical)).expect("canonical file exists");
        let copy = std::fs::read(editors().join(copy)).expect("copied file exists");
        assert_eq!(canonical, copy, "the generated copies must not drift");
    }
    let expected: Vec<&[u8]> = vec![textmate.as_bytes(), vim.as_bytes(), emacs.as_bytes()];
    for (path, bytes) in [
        ("grammars/yashy.tmLanguage.json", expected[0]),
        ("grammars/yashy.vim", expected[1]),
        ("grammars/yashy.el", expected[2]),
    ] {
        let committed = std::fs::read(editors().join(path)).expect("generated file exists");
        assert_eq!(committed, bytes, "{path} does not match its renderer");
    }
}

#[test]
fn a_new_keyword_requires_regeneration() {
    let model = vocabulary::model();
    let mut changed = model.clone();
    changed.keywords.push("example_future_keyword");
    assert_ne!(
        grammar::render_textmate(&model),
        grammar::render_textmate(&changed)
    );
    assert_ne!(grammar::render_vim(&model), grammar::render_vim(&changed));
    assert_ne!(
        grammar::render_emacs(&model),
        grammar::render_emacs(&changed)
    );
    assert_ne!(
        grammar::render_tree_sitter(&model),
        grammar::render_tree_sitter(&changed)
    );
}

#[test]
fn every_vocabulary_word_has_exactly_one_highlight_class() {
    let model = vocabulary::model();
    let textmate = grammar::render_textmate(&model);
    let vim = grammar::render_vim(&model);
    let emacs = grammar::render_emacs(&model);
    let tree_sitter = grammar::render_tree_sitter(&model);
    for word in model
        .keywords
        .iter()
        .chain(model.types.iter())
        .chain(model.operators.iter())
        .chain(model.delimiters.iter())
    {
        let json_escaped = escaped(word).replace('\\', "\\\\");
        assert!(
            textmate.contains(word)
                || textmate.contains(&escaped(word))
                || textmate.contains(&json_escaped),
            "{word} is missing from TextMate"
        );
        assert!(
            tree_sitter.contains(word) || tree_sitter.contains(&escaped(word)),
            "{word} is missing from tree-sitter"
        );
        assert!(
            emacs.contains(&format!("\"{word}\"")),
            "{word} is missing from Emacs"
        );
    }
    for word in &model.keywords {
        assert!(
            vim.split_whitespace().any(|part| part == *word),
            "{word} is missing from Vim"
        );
    }
    // One scope per class, not one per token: a keyword cannot be a type.
    assert_eq!(
        textmate.matches("\"keyword.control.yashy\"").count(),
        1,
        "keywords have exactly one TextMate scope"
    );
    assert_eq!(
        textmate.matches("\"support.type.yashy\"").count(),
        1,
        "types have exactly one TextMate scope"
    );
    assert_eq!(
        textmate.matches("\"keyword.operator.yashy\"").count(),
        1,
        "operators have exactly one TextMate scope"
    );
    assert_eq!(
        textmate.matches("\"punctuation.yashy\"").count(),
        1,
        "delimiters have exactly one TextMate scope"
    );
}

#[test]
fn the_two_gated_clients_wire_the_compiler_verbs() {
    let vscode = std::fs::read_to_string(editors().join("vscode/package.json"))
        .expect("the VS Code manifest exists");
    let extension = std::fs::read_to_string(editors().join("vscode/src/extension.ts"))
        .expect("the VS Code extension exists");
    let neovim = std::fs::read_to_string(editors().join("neovim/lua/yashy/init.lua"))
        .expect("the Neovim module exists");
    for text in [&vscode, &extension] {
        assert!(
            text.contains("lsp"),
            "the VS Code client must start the LSP"
        );
        assert!(text.contains("--stdio"), "the transport is stdio");
    }
    assert!(vscode.contains("taskDefinitions"), "tasks are contributed");
    assert!(extension.contains("'task'"), "tasks are compiler verbs");
    assert!(extension.contains("'test'"), "tests are compiler verbs");
    assert!(
        vscode.contains("debug"),
        "the DAP configuration is contributed"
    );
    assert!(
        neovim.contains("\"lsp\", \"--stdio\""),
        "Neovim starts the LSP"
    );
    assert!(neovim.contains("check"), "`:make` uses `yashyc check`");
    assert!(neovim.contains("task"), "named tasks are available");
}

#[test]
fn the_four_documented_clients_point_at_the_lsp() {
    for (file, needle) in [
        ("grammars/helix/languages.toml", "yashyc"),
        ("zed/extension.toml", "yashy"),
        ("zed/src/lib.rs", "lsp"),
        ("jetbrains/README.md", "yashyc lsp --stdio"),
        ("emacs/yashy.el", "yashy-mode"),
        ("README.md", "tokens --grammar"),
    ] {
        let path = editors().join(file);
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("{} must exist: {error}", path.display()));
        assert!(text.contains(needle), "{file} must mention `{needle}`");
    }
}
