Register-ArgumentCompleter -Native -CommandName 'yashyc' -ScriptBlock {
  param($wordToComplete)
  @('build', 'run', 'check', 'verify', 'test', 'serve', 'watch', 'cache', 'explain', 'init', 'new', 'fmt', 'completions', 'help', 'doctor', 'lsp', 'mcp', 'doc', 'profile', 'lint', 'fix', 'inspect', 'tasks', 'task', 'expand', 'index', 'outline', 'graph', 'search', 'context', 'lock', 'fetch', 'update', 'add', 'audit', 'publish', 'playground', 'man', 'version') | Where-Object { $_ -like "$wordToComplete*" }
}
