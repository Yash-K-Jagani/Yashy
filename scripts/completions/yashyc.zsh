#compdef yashyc
_yashyc() {
  local -a verbs flags
  verbs=('build' 'run' 'check' 'verify' 'test' 'serve' 'watch' 'cache' 'explain' 'init' 'new' 'fmt' 'completions' 'help' 'doctor' 'lsp' 'mcp' 'doc' 'profile' 'lint' 'fix' 'inspect' 'tasks' 'task' 'expand' 'index' 'outline' 'graph' 'search' 'context' 'lock' 'fetch' 'update' 'add' 'audit' 'publish' 'playground' 'man' 'version')
  flags=('-o' '-O0' '-O1' '-O2' '-O3' '--release' '--timings' '--jobs' '--target' '--emit-ir' '--opt-report' '--no-verify' '--reproducible' '--debug' '--sandbox' '--json' '--deny' '--allow' '--inventory' '--filter' '--list' '--run' '--test' '--interval' '--all' '--template' '-w' '--check' '--stdio' '--serve' '--port' '--sample' '--docs' '--group' '--dry-run' '--human' '--dot' '--registry' '--task' '--budget' '--path' '--git' '--rev' '--host' '--color' '--no-color')
  if (( CURRENT == 2 )); then
    _describe 'verb' verbs
  else
    _describe 'flag' flags
  fi
}
compdef _yashyc yashyc
