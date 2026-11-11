#!/usr/bin/env sh
# Run the runnable examples of tutorial chapters with a given Yashy binary.
#
# Usage:
#   scripts/run-tutorial-blocks.sh <yashy> <chapter.md>...
#
# The contract is the tutorial harness's: a ```yashy block is a program, and
# the `// =>` lines inside it are the stdout it must print. A block with no
# expected output only has to compile and run.
#
# This exists so the release's macOS install test can check the tutorial with
# the *installed* compiler, on a runner that has no Rust toolchain: the Rust
# harness cannot run there, but the compiler can.
#
# A block that names its files (`// file: main.ya`) is a multi-file project
# rather than a standalone program, and is skipped here: the Rust harness
# builds those as projects, and a shell script has no project loader.

set -eu

die() {
    printf 'FAIL: %s\n' "$*" >&2
    exit 1
}

ok() {
    printf 'ok - %s\n' "$*"
}

[ "$#" -ge 2 ] || {
    printf 'usage: scripts/run-tutorial-blocks.sh <yashy> <chapter.md>...\n' >&2
    exit 2
}

yashy=$1
shift
[ -x "$yashy" ] || die "no yashy binary at $yashy"

tmp=$(mktemp -d "${TMPDIR:-/tmp}/yashy-tutorial.XXXXXX")
cleanup() {
    rm -rf "$tmp"
}
trap cleanup 0 1 2 15

blocks=0
skipped=0

for chapter in "$@"; do
    [ -f "$chapter" ] || die "no chapter at $chapter"
    name=$(basename -- "$chapter")
    rm -f "$tmp"/block-*.ya "$tmp"/block-*.expected
    # One pass over the chapter: every ```yashy fence becomes a pair of files,
    # the program and the stdout it must print.
    awk -v dir="$tmp" '
        function flush(   file) {
            if (!inside) {
                return
            }
            n++
            file = dir "/block-" n ".ya"
            printf "%s", code > file
            close(file)
            file = dir "/block-" n ".expected"
            printf "%s", expected > file
            close(file)
        }
        /^[[:space:]]*```yashy/ {
            inside = 1
            code = ""
            expected = ""
            next
        }
        inside && /^[[:space:]]*```[[:space:]]*$/ {
            flush()
            inside = 0
            next
        }
        inside {
            code = code $0 "\n"
            if (match($0, /^[[:space:]]*\/\/ =>/)) {
                text = substr($0, RSTART + RLENGTH)
                sub(/^ /, "", text)
                expected = expected text "\n"
            }
        }
        END { flush() }
    ' "$chapter"

    for source in "$tmp"/block-*.ya; do
        [ -f "$source" ] || continue
        blocks=$((blocks + 1))
        expected_file="${source%.ya}.expected"
        if grep -q '^[[:space:]]*// file:' "$source"; then
            skipped=$((skipped + 1))
            continue
        fi
        exe="${source%.ya}.bin"
        if ! "$yashy" build "$source" -o "$exe" >"$tmp/build.out" 2>&1; then
            printf '%s\n' "--- $name: block $blocks did not compile ---" >&2
            cat "$tmp/build.out" >&2
            exit 1
        fi
        if ! "$exe" >"$tmp/run.out" 2>"$tmp/run.err"; then
            printf '%s\n' "--- $name: block $blocks did not run ---" >&2
            cat "$tmp/run.err" >&2
            exit 1
        fi
        if [ -s "$expected_file" ]; then
            if ! cmp -s "$expected_file" "$tmp/run.out"; then
                printf '%s\n' "--- $name: block $blocks printed the wrong output ---" >&2
                printf 'expected:\n' >&2
                cat "$expected_file" >&2
                printf 'actual:\n' >&2
                cat "$tmp/run.out" >&2
                exit 1
            fi
        fi
    done
    ok "$name"
done

printf 'ok - %d tutorial blocks ran (%d multi-file blocks skipped)\n' "$blocks" "$skipped"
