_yashyc() {
  local cur=${COMP_WORDS[COMP_CWORD]}
  local verbs='build run check verify test serve watch cache explain init new debug repl fmt completions help doctor lsp mcp doc profile tokens trace lint fix inspect tasks task expand index outline graph search context lock fetch update add audit sbom verify-package self-update attest verify-attestation publish playground man version'
  local flags='-o -O0 -O1 -O2 -O3 --release --timings --debug-info --features --no-default-features --jobs --target --emit-ir --opt-report --no-verify --reproducible --debug --sandbox --json --deny --allow --inventory --workspace -p --filter --list --run --test --interval --all --template --break --binary -w --check --stdio --serve --port --sample --report --alloc-report --grammar --bpe --fail-orphans --docs --group --dry-run --human --addr --layout --dot --registry --task --budget --path --git --rev --capabilities --format --archive --signature --key-id --material --attestation --host --color --no-color --pager --no-pager --hyperlink --no-hyperlink'
  if (( COMP_CWORD == 1 )); then
    COMPREPLY=( $(compgen -W "$verbs" -- "$cur") )
  else
    COMPREPLY=( $(compgen -W "$flags" -- "$cur") )
  fi
}
complete -F _yashyc yashyc
