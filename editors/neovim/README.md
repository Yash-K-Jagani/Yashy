# Yashy for Neovim

Requires Neovim 0.10 or newer: the setup uses `vim.lsp.start`.

```lua
require('yashy').setup({ compiler = 'yashyc' })
```

Put `editors/neovim/lua` on your `runtimepath` (or install this directory as a
plugin). The module:

- starts `yashyc lsp --stdio` for every `.ya` buffer, with `yashy.toml` as the
  root marker;
- sets `:make` to `yashyc check` with a quickfix error format;
- adds `:YashyTask <name>` for tasks declared in `yashy.toml`.

Highlighting uses the generated Vim syntax file in `editors/vim/yashy.vim` by
default. When the generated tree-sitter grammar under
`editors/grammars/tree-sitter-yashy` is installed, add it as a parser for the
`yashy` file type:

```lua
vim.treesitter.language.register('yashy', 'yashy')
```

Debug uses Neovim's DAP client with adapter `yashyc debug`; the compiler writes
the LLDB script the debugger needs. No editor-only parser is involved.

Regenerate all syntax files after changing the lexer:

```text
yashyc tokens --grammar editors
```
