# Editor setup

`yashy lsp` is the whole configuration: the server is a sibling binary
(`yashy-lsp` beside the `yashy` executable), launched on stdio. There is
no second thing to install or keep in step. When the sibling is missing,
`yashy lsp` exits 2 naming the path, and `yashy doctor` reports the lookup
in the place a user runs when something is wrong.

## Neovim

```lua
vim.lsp.config('yashy', {
  cmd = { 'yashy', 'lsp' },
  filetypes = { 'yashy' },
  root_markers = { 'yashy.toml' },
})
vim.lsp.enable('yashy')
```

## VS Code

```json
{
  "languages": [
    {
      "id": "yashy",
      "aliases": ["Yashy", "yashy"],
      "extensions": [".ya"]
    }
  ],
  "grammars": [
    {
      "language": "yashy",
      "scopeName": "source.yashy",
      "path": "./syntaxes/yashy.tmLanguage.json"
    }
  ]
}
```

```json
{
  "yashy.lsp.command": ["yashy", "lsp"]
}
```

The TextMate grammar is `editors/vscode/syntaxes/yashy.tmLanguage.json`;
Vim and Emacs definitions live beside it in `editors/`. The three
definitions are kept in step with the lexer's keyword list by
`crates/yashy-lsp/tests/grammar.rs`, so adding a keyword without a grammar
entry fails the test suite.

## Helix

```toml
[[language]]
name = "yashy"
scope = "source.yashy"
file-types = ["ya"]

[language-server.yashy]
command = "yashy"
args = ["lsp"]
```
