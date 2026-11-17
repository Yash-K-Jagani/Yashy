# Yashy for JetBrains IDEs

JetBrains does not ship a first-class arbitrary-LSP client in every product, so
this directory documents the configuration rather than bundling a second
compiler or a handwritten parser.

## LSP via the JetBrains LSP client plugin

Install an LSP client plugin that supports custom languages, then add a
configuration with:

- language id: `Yashy`
- file extension: `.ya`
- server command: `yashyc lsp --stdio`
- root marker: `yashy.toml`
- completion, hover, go to definition, rename, formatting, diagnostics: enabled

## Everything else is a compiler verb

| Action | Command |
|---|---|
| Check | `yashyc check <file.ya>` |
| Test | `yashyc test` |
| Format | `yashyc fmt -w <file.ya>` |
| Named task | `yashyc task <name>` |
| Explain a diagnostic | `yashyc explain <code>` |
| Doctor | `yashyc doctor` |

Syntax highlighting uses the generated TextMate grammar at
`editors/grammars/yashy.tmLanguage.json`. If your IDE can load a TextMate
bundle, point it at that file. Do not edit it by hand: regenerate with

```text
yashyc tokens --grammar editors
```
