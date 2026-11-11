#!/usr/bin/env sh
# Write the book's language reference from the compiler's own index.
#
# Usage:
#   scripts/book-reference.sh [--out <file>]
#
# The reference is generated, not written by hand: `yashy index --json` reads
# the same tables the compiler checks a program against, so a signature in the
# book cannot drift from the signature in the code. The CI job regenerates the
# page and fails when the file differs, which is what makes the generation a
# gate rather than a suggestion.

set -eu

die() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"

out="book/src/reference.md"
while [ "$#" -gt 0 ]; do
    case "$1" in
        --out)
            [ "$#" -ge 2 ] || die "--out needs a value"
            out=$2
            shift 2
            ;;
        -h|--help)
            printf '%s\n' "usage: scripts/book-reference.sh [--out <file>]"
            exit 0
            ;;
        *)
            die "unknown argument: $1"
            ;;
    esac
done

if [ -x target/release/yashy ]; then
    yashy=target/release/yashy
elif [ -x target/debug/yashy ]; then
    yashy=target/debug/yashy
else
    die "no yashy binary; run: cargo build --release -p yashy-driver"
fi

index=$("$yashy" index --json crates/yashy-stdlib/yashy) ||
    die "the index failed"

# The symbols, one per line, grouped by the module they come from. The
# compiler's JSON is read with awk rather than a JSON parser, because the
# fields the reference needs -- the name, the kind, the signature and the
# module -- are the ones the index prints one per line.
printf '%s\n' "$index" | awk '
    /"name":/ { name = $0; sub(/^.*"name": "/, "", name); sub(/",?$/, "", name); next }
    /"kind":/ { kind = $0; sub(/^.*"kind": "/, "", kind); sub(/",?$/, "", kind); next }
    /"file":/ { file = $0; sub(/^.*"file": "/, "", file); sub(/",?$/, "", file); next }
    /"signature":/ {
        signature = $0
        sub(/^.*"signature": "/, "", signature)
        sub(/",?$/, "", signature)
        module = file
        sub(/^.*\//, "", module)
        sub(/\.ya$/, "", module)
        if (kind == "fn") {
            printf "%s\t%s\t%s\n", module, name, signature
        }
    }
' | sort -u > "$out.symbols"

{
    printf '# The language reference\n\n'
    printf 'Generated from the standard library by `scripts/book-reference.sh`.\n'
    printf 'Every signature here is the one the compiler checks a program against;\n'
    printf 'the CI job regenerates the page and fails when it differs.\n\n'
    printf 'The standard library is documented in [the reference page](stdlib.md).\n\n'
    printf '## The standard library\n\n'

    module=""
    while IFS="$(printf '\t')" read -r current name signature; do
        [ -n "$current" ] || continue
        if [ "$current" != "$module" ]; then
            module=$current
            printf '### `%s`\n\n' "$module"
        fi
        printf '%s\n' "- \`$signature\`"
    done < "$out.symbols"
} > "$out"

rm -f "$out.symbols"
printf '%s\n' "$out"
