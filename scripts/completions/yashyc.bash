_yashyc() {
  local cur=${COMP_WORDS[COMP_CWORD]}
  local verbs='build run check verify test bench serve watch cache explain init new debug repl fmt completions help doctor lsp mcp doc profile coverage tokens trace lint fix inspect tasks task expand index outline graph search context lock fetch update add audit sbom verify-package self-update outdated vendor tree why info yank remove attest verify-attestation publish playground man version'
  local flags='-o -O0 -O1 -O2 -O3 --release --timings --debug-info --print-capabilities --features --no-default-features --jobs --target --emit-ir --opt-report --no-verify --reproducible --debug --sandbox --json --deny --allow --print-cfg --inventory --workspace -p --filter --list --run --test --interval --all --template --break --binary -w --check --stdio --serve --port --sample --report --alloc-report --format --fail-under --grammar --bpe --fail-orphans --events --docs --group --dry-run --human --addr --layout --dot --registry --task --budget --path --git --rev --capabilities --archive --signature --depth --reason --key-id --material --attestation --rebuild --host --color --no-color --pager --no-pager --hyperlink --no-hyperlink'
  if (( COMP_CWORD == 1 )); then
    COMPREPLY=( $(compgen -W "$verbs" -- "$cur") )
  else
    COMPREPLY=( $(compgen -W "$flags" -- "$cur") )
  fi
}
complete -F _yashyc yashyc
