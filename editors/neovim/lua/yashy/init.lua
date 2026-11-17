-- Yashy support for Neovim (0.10+).
--
-- The LSP client is Neovim's own; the server is the compiler. This file never
-- parses Yashy itself. Syntax highlighting comes from the generated tree-sitter
-- grammar when the parser is installed, and from the generated Vim syntax file
-- otherwise.

local M = {}

local function root_dir(fname)
  return vim.fs.dirname(vim.fs.find({ "yashy.toml", ".git" }, { upward = true, path = fname })[1])
end

function M.setup(opts)
  opts = opts or {}
  local compiler = opts.compiler or "yashyc"

  vim.api.nvim_create_autocmd("FileType", {
    pattern = "yashy",
    callback = function(args)
      local client = vim.lsp.start({
        name = "yashy",
        cmd = { compiler, "lsp", "--stdio" },
        root_dir = root_dir(args.file) or vim.fn.getcwd(),
        settings = {},
      }, {
        bufnr = args.buf,
        reuse_client = function(other, config)
          return other.name == config.name and other.root_dir == config.root_dir
        end,
      })
      if client then
        vim.lsp.buf_attach_client(args.buf, client.id)
      end
    end,
  })

  vim.api.nvim_create_autocmd("BufRead,BufNewFile", {
    pattern = "*.ya",
    callback = function()
      vim.bo.filetype = "yashy"
    end,
  })

  -- `:make` is the compiler's check verb, with diagnostics in the quickfix
  -- list. A task from yashy.toml is run with `:YashyTask <name>`.
  vim.api.nvim_create_autocmd("FileType", {
    pattern = "yashy",
    callback = function()
      vim.bo.makeprg = compiler .. " check"
      vim.bo.errorformat = "%E%f:%l:%c: error: %m,%W%f:%l:%c: warning: %m,%C%.%#"
    end,
  })
  vim.api.nvim_create_user_command("YashyTask", function(command)
    vim.cmd("!" .. compiler .. " task " .. vim.fn.shellescape(command.args))
  end, { nargs = 1 })
end

return M
