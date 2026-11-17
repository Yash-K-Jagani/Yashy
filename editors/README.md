# Editor support

Every editor here reads the same two things: a **generated grammar** from the
lexer's own tables, and **the compiler's LSP server**. There is no
editor-only parser and no second definition of a keyword.

## One grammar, six clients

`yashyc tokens --grammar editors` rewrites every generated file:

| Generated file | Used by |
|---|---|
| `grammars/yashy.tmLanguage.json` | VS Code, Helix, Zed, JetBrains (TextMate) |
| `grammars/yashy.vim` | Vim |
| `grammars/yashy.el` | Emacs |
| `grammars/helix/languages.toml` | Helix |
| `grammars/tree-sitter-yashy/` | Neovim, Zed |
| `vscode/syntaxes/yashy.tmLanguage.json` | VS Code (copy of the canonical grammar) |
| `vim/yashy.vim`, `emacs/yashy.el` | copies of the canonical modes |

The files under `grammars/` are canonical; the copies in the client folders are
byte-identical. `cargo test -p yashy-driver --test grammar` compares all of them
with the renderer, so a new keyword cannot appear in one grammar and be missing
from another.

## Gated clients

VS Code and Neovim are the two clients the project tests: each drives the same
LSP transcript through completion, hover, rename, formatting and a breakpoint.

| Client | Setup |
|---|---|
| [VS Code](vscode/README.md) | `vscode-languageclient`, commands, tasks, DAP, snippets |
| [Neovim](neovim/README.md) | `vim.lsp.start`, `:make`, `:YashyTask`, tree-sitter |

## Documented clients

Helix uses `grammars/helix/languages.toml` directly. Emacs loads
`emacs/yashy.el`. Zed uses `zed/` with the generated tree-sitter grammar and
`yashyc lsp --stdio`. JetBrains uses the LSP client plugin described in
[jetbrains/README.md](jetbrains/README.md). These validate when the editor's
headless CLI is available; none of them bundles a compiler.

## Regenerating

Add the token to `crates/yashy-lexer/src/token.rs`, then:

```text
yashyc tokens --grammar editors
cargo test -p yashy-driver --test grammar
cargo test -p yashy-lsp --test grammar
```

`yashyc tokens <file.ya> --json` prints the lexer's view of a file with byte
spans, which is the fastest way to see what a grammar rule should match.
