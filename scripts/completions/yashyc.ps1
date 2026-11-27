Register-ArgumentCompleter -Native -CommandName 'yashyc' -ScriptBlock {
  param($wordToComplete)
  @('build', 'run', 'check', 'verify', 'test', 'serve', 'watch', 'cache', 'explain', 'init', 'new', 'debug', 'repl', 'fmt', 'completions', 'help', 'doctor', 'lsp', 'mcp', 'doc', 'profile', 'coverage', 'tokens', 'trace', 'bench', 'fuzz', 'mutate', 'lint', 'fix', 'migrate', 'inspect', 'tasks', 'task', 'expand', 'index', 'outline', 'graph', 'search', 'context', 'conformance', 'review', 'impact', 'agent', 'lock', 'fetch', 'update', 'add', 'audit', 'sbom', 'verify-package', 'self-update', 'outdated', 'vendor', 'tree', 'why', 'info', 'yank', 'remove', 'attest', 'verify-attestation', 'publish', 'playground', 'man', 'version') | Where-Object { $_ -like "$wordToComplete*" }
}
