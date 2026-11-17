_yashyc() {
  local cur=${COMP_WORDS[COMP_CWORD]}
  local verbs='build run check verify test serve watch cache explain init new fmt completions help doctor lsp mcp doc profile lint fix inspect tasks task expand index outline graph search context lock fetch update add audit publish playground man version'
  local flags='-o -O0 -O1 -O2 -O3 --release --timings --jobs --target --emit-ir --opt-report --no-verify --reproducible --debug --sandbox --json --deny --allow --inventory --filter --list --run --test --interval --all --template -w --check --stdio --serve --port --sample --docs --group --dry-run --human --dot --registry --task --budget --path --git --rev --host --color --no-color'
  if (( COMP_CWORD == 1 )); then
    COMPREPLY=( $(compgen -W "$verbs" -- "$cur") )
  else
    COMPREPLY=( $(compgen -W "$flags" -- "$cur") )
  fi
}
complete -F _yashyc yashyc
