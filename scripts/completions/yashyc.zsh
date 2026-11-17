#compdef yashyc
_yashyc() {
  local -a verbs flags
  verbs=('build' 'run' 'check' 'verify' 'test' 'serve' 'watch' 'cache' 'explain' 'init' 'new' 'debug' 'repl' 'fmt' 'completions' 'help' 'doctor' 'lsp' 'mcp' 'doc' 'profile' 'tokens' 'trace' 'lint' 'fix' 'inspect' 'tasks' 'task' 'expand' 'index' 'outline' 'graph' 'search' 'context' 'lock' 'fetch' 'update' 'add' 'audit' 'sbom' 'verify-package' 'self-update' 'outdated' 'vendor' 'tree' 'why' 'info' 'yank' 'remove' 'attest' 'verify-attestation' 'publish' 'playground' 'man' 'version')
  flags=('-o' '-O0' '-O1' '-O2' '-O3' '--release' '--timings' '--debug-info' '--print-capabilities' '--features' '--no-default-features' '--jobs' '--target' '--emit-ir' '--opt-report' '--no-verify' '--reproducible' '--debug' '--sandbox' '--json' '--deny' '--allow' '--inventory' '--workspace' '-p' '--filter' '--list' '--run' '--test' '--interval' '--all' '--template' '--break' '--binary' '-w' '--check' '--stdio' '--serve' '--port' '--sample' '--report' '--alloc-report' '--grammar' '--bpe' '--fail-orphans' '--docs' '--group' '--dry-run' '--human' '--addr' '--layout' '--dot' '--registry' '--task' '--budget' '--path' '--git' '--rev' '--capabilities' '--format' '--archive' '--signature' '--depth' '--reason' '--key-id' '--material' '--attestation' '--host' '--color' '--no-color' '--pager' '--no-pager' '--hyperlink' '--no-hyperlink')
  if (( CURRENT == 2 )); then
    _describe 'verb' verbs
  else
    _describe 'flag' flags
  fi
}
compdef _yashyc yashyc
