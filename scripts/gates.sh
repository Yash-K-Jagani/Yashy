#!/usr/bin/env sh
# Run every gate and print a table.
#
# Usage:
#   scripts/gates.sh [--json]
#
# Each gate is a command with a budget. The table says what passed, what
# failed and how long each took; `--json` prints the same facts as a
# document, so a nightly job can read them without parsing text.
#
# The exit status is 1 when any gate failed. The runner stops at the first
# failure unless `--all` is given: a gate that needs the compiler built is
# not worth running after the build failed.

set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"

json=0
keep_going=0
while [ "$#" -gt 0 ]; do
    case "$1" in
        --json) json=1; shift ;;
        --all) keep_going=1; shift ;;
        -h|--help)
            printf '%s\n' "usage: scripts/gates.sh [--json] [--all]"
            exit 0
            ;;
        *)
            printf 'error: unknown argument: %s\n' "$1" >&2
            exit 2
            ;;
    esac
done

# The compiler the gates drive: the release build when it is there.
if [ -x target/release/yashy ]; then
    yashy="$root/target/release/yashy"
elif [ -x target/debug/yashy ]; then
    yashy="$root/target/debug/yashy"
else
    yashy=""
fi

# name | budget | command
gates='build|10m|cargo build --release --workspace
check|5m|cargo check --workspace --all-targets
tests|45m|cargo test --workspace --no-fail-fast
format|2m|FORMAT
reproducible|5m|REPRO
sandbox|5m|cargo test -p yashy-driver --test sandbox_escape
never-panics|5m|cargo test -p yashy-driver --test never_panics
book|10m|BOOK'

run_gate() {
    name=$1
    budget=$2
    command=$3
    started=$(date +%s)
    if eval "$command" >"$root/.gates-$name.log" 2>&1; then
        status=passed
        code=0
    else
        status=failed
        code=1
    fi
    ended=$(date +%s)
    elapsed=$((ended - started))
    printf '%-16s %-8s %-6s %s\n' "$name" "$status" "${elapsed}s" "$budget" >&2
    if [ "$json" -eq 1 ]; then
        printf '  {"gate": "%s", "status": "%s", "seconds": %s, "budget": "%s"}\n' \
            "$name" "$status" "$elapsed" "$budget"
    fi
    if [ "$code" -ne 0 ] && [ "$keep_going" -eq 0 ]; then
        printf '\nthe %s gate failed; its output is in .gates-%s.log\n' "$name" "$name" >&2
        return 1
    fi
    return $code
}

if [ "$json" -eq 1 ]; then
    printf '{\n  "schema_version": 1,\n  "tool": "gates",\n  "gates": [\n'
fi

printf '%-16s %-8s %-6s %s\n' "gate" "status" "time" "budget" >&2
failed=0
while IFS='|' read -r name budget command; do
    [ -n "$name" ] || continue
    case "$command" in
        FORMAT)
            if [ -z "$yashy" ]; then
                command="echo 'no yashy binary; run cargo build --release -p yashy-driver'"
            else
                command="find crates/yashy-stdlib/yashy packages examples -name '*.ya' -print0 | xargs -0 '$yashy' fmt --check"
            fi
            ;;
        REPRO)
            if [ -z "$yashy" ]; then
                command="echo 'no yashy binary; run cargo build --release -p yashy-driver'"
            else
                command="sh scripts/repro-check.sh"
            fi
            ;;
        BOOK)
            if [ -z "$yashy" ]; then
                command="echo 'no yashy binary; run cargo build --release -p yashy-driver'"
            else
                command="sh scripts/run-tutorial-blocks.sh '$yashy' book/src/getting-started.md book/src/verification-guide.md book/src/tooling.md book/src/cookbook.md && sh scripts/book-reference.sh && git diff --exit-code book/src/reference.md"
            fi
            ;;
    esac
    run_gate "$name" "$budget" "$command" || failed=1
done <<EOF
$gates
EOF

if [ "$json" -eq 1 ]; then
    printf '  ],\n  "ok": %s\n}\n' "$([ "$failed" -eq 0 ] && echo true || echo false)"
fi

if [ "$failed" -ne 0 ]; then
    printf '\nsome gates failed\n' >&2
    exit 1
fi
printf '\nevery gate passed\n' >&2
