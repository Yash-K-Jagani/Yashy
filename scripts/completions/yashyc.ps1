Register-ArgumentCompleter -Native -CommandName 'yashyc' -ScriptBlock {
  param($wordToComplete)
  @('build', 'run', 'check', 'verify', 'test', 'serve', 'watch', 'cache', 'explain', 'init', 'new', 'debug', 'repl', 'fmt', 'completions', 'help', 'doctor', 'lsp', 'mcp', 'doc', 'profile', 'tokens', 'trace', 'lint', 'fix', 'inspect', 'tasks', 'task', 'expand', 'index', 'outline', 'graph', 'search', 'context', 'lock', 'fetch', 'update', 'add', 'audit', 'sbom', 'verify-package', 'self-update', 'outdated', 'vendor', 'tree', 'why', 'info', 'yank', 'remove', 'attest', 'verify-attestation', 'publish', 'playground', 'man', 'version') | Where-Object { $_ -like "$wordToComplete*" }
}
