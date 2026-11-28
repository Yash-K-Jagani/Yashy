#compdef yashyc
_yashyc() {
  local -a verbs flags
  verbs=('build' 'run' 'check' 'verify' 'test' 'serve' 'watch' 'cache' 'explain' 'init' 'new' 'debug' 'repl' 'fmt' 'completions' 'help' 'doctor' 'lsp' 'mcp' 'doc' 'profile' 'coverage' 'tokens' 'trace' 'bench' 'fuzz' 'mutate' 'lint' 'fix' 'migrate' 'inspect' 'tasks' 'task' 'expand' 'index' 'outline' 'graph' 'search' 'context' 'advisory' 'conformance' 'review' 'impact' 'agent' 'lock' 'fetch' 'update' 'add' 'audit' 'sbom' 'verify-package' 'self-update' 'outdated' 'vendor' 'tree' 'why' 'info' 'yank' 'remove' 'attest' 'verify-attestation' 'publish' 'playground' 'man' 'version')
  flags=('-o' '-O0' '-O1' '-O2' '-O3' '--release' '--timings' '--debug-info' '--print-capabilities' '--features' '--no-default-features' '--jobs' '--target' '--emit-ir' '--emit' '--header' '--opt-report' '--stack-report' '--no-verify' '--reproducible' '--debug' '--sandbox' '--json' '--deny' '--allow' '--print-cfg' '--sarif' '--inventory' '--require-inventory' '--certificates' '--workspace' '-p' '--allow-path' '--filter' '--affected' '--list' '--run' '--test' '--interval' '--all' '--catalog' '--template' '--break' '--binary' '-w' '--check' '--stdio' '--serve' '--port' '--private' '--out' '--versioned' '--version' '--sample' '--report' '--alloc-report' '--format' '--fail-under' '--tests' '--grammar' '--bpe' '--markdown' '--fail-orphans' '--events' '--check-baseline' '--history' '--append' '--thresholds' '--measure' '--compare' '--save-baseline' '--key' '--iterations' '--warmup' '--findings' '--replay' '--score' '--docs' '--group' '--dry-run' '--human' '--addr' '--layout' '--dot' '--from' '--depth' '--edges' '--registry' '--task' '--budget' '--expect-revision' '--mode' '--focus' '--changed' '--diagnostic' '--feed' '--keyring' '--package' '--baseline' '--verify' '--root' '--tree' '--patch' '--verbs' '--ttl' '--nonce' '--path' '--git' '--rev' '--capabilities' '--archive' '--signature' '--reason' '--key-id' '--material' '--attestation' '--rebuild' '--host' '--color' '--no-color' '--pager' '--no-pager' '--hyperlink' '--no-hyperlink')
  if (( CURRENT == 2 )); then
    _describe 'verb' verbs
  else
    _describe 'flag' flags
  fi
}
compdef _yashyc yashyc
