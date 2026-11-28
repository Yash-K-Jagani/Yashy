_yashyc() {
  local cur=${COMP_WORDS[COMP_CWORD]}
  local verbs='build run check verify test serve watch cache explain init new debug repl fmt completions help doctor lsp mcp doc profile coverage tokens trace bench fuzz mutate lint fix migrate inspect tasks task expand index outline graph search context advisory conformance review impact agent lock fetch update add audit sbom verify-package self-update outdated vendor tree why info yank remove attest verify-attestation publish playground man version'
  local flags='-o -O0 -O1 -O2 -O3 --release --timings --debug-info --print-capabilities --features --no-default-features --jobs --target --emit-ir --emit --header --check --opt-report --stack-report --no-verify --reproducible --debug --sandbox --json --deny --allow --print-cfg --sarif --inventory --require-inventory --certificates --workspace -p --allow-path --filter --affected --list --run --test --interval --all --catalog --template --break --binary -w --stdio --serve --port --private --out --versioned --version --sample --report --alloc-report --format --fail-under --tests --grammar --bpe --markdown --fail-orphans --events --check-baseline --history --append --thresholds --measure --compare --save-baseline --key --iterations --warmup --findings --replay --score --docs --group --dry-run --source --human --addr --layout --dot --from --depth --edges --registry --task --budget --expect-revision --mode --focus --changed --diagnostic --feed --fetch --offline --pin --max-age --keyring --package --baseline --verify --root --tree --patch --verbs --ttl --nonce --key-id --path --git --rev --capabilities --archive --signature --reason --material --attestation --rebuild --host --color --no-color --pager --no-pager --hyperlink --no-hyperlink'
  if (( COMP_CWORD == 1 )); then
    COMPREPLY=( $(compgen -W "$verbs" -- "$cur") )
  else
    COMPREPLY=( $(compgen -W "$flags" -- "$cur") )
  fi
}
complete -F _yashyc yashyc
