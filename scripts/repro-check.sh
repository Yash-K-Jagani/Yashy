#!/usr/bin/env sh
# Build the same source twice, in two directories, and compare the bytes.
#
# Usage:
#   scripts/repro-check.sh [source.ya]
#
# Without a source, examples/hello/main.ya is used. The script exits 0 when
# the two binaries are identical, and 1 with the first differing byte offset
# when they are not.
#
# The offset matters: "the builds differ" is not an answer a person can act
# on, and the offset points at the byte that carries the difference.

set -eu

die() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"

source_file=${1:-examples/hello/main.ya}
[ -f "$source_file" ] || die "no source at $source_file"

# The compiler: the one in target/release if it is there, the debug one
# otherwise. A release build is what a release would ship, so it is the one
# worth checking.
if [ -x target/release/yashy ]; then
    yashy="$root/target/release/yashy"
elif [ -x target/debug/yashy ]; then
    yashy="$root/target/debug/yashy"
else
    die "no yashy binary; run: cargo build --release -p yashy-driver"
fi

work=$(mktemp -d "${TMPDIR:-/tmp}/yashy-repro.XXXXXX")
cleanup() {
    rm -rf "$work"
}
trap cleanup 0 1 2 15

mkdir -p "$work/a" "$work/b"
cp "$source_file" "$work/a/main.ya"
cp "$source_file" "$work/b/main.ya"

for side in a b; do
    (cd "$work/$side" && "$yashy" build main.ya -o "hello" --reproducible) >/dev/null ||
        die "the build in $work/$side failed"
done

hash_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | awk '{print $1}'
    else
        die "no SHA-256 tool found; install sha256sum (coreutils) or shasum"
    fi
}

first="$work/a/hello"
second="$work/b/hello"
left=$(hash_of "$first")
right=$(hash_of "$second")

if [ "$left" = "$right" ]; then
    printf 'ok - both builds are %s\n' "$left"
    exit 0
fi

# The first byte at which they differ, so the failure names a place.
offset=$(cmp "$first" "$second" 2>&1 | sed -n 's/.*byte \([0-9]*\),.*/\1/p' | head -n 1)
if [ -z "$offset" ]; then
    offset=$(cmp -l "$first" "$second" 2>/dev/null | awk 'NR == 1 { print $1 - 1 }')
fi
printf 'FAIL: the two builds differ\n' >&2
printf '  first:  %s\n' "$left" >&2
printf '  second: %s\n' "$right" >&2
if [ -n "$offset" ]; then
    printf '  first differing byte: %s\n' "$offset" >&2
fi
exit 1
