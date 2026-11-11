#!/usr/bin/env sh
# Build the static registry index from the first-party packages.
#
# Usage:
#   scripts/publish-registry.sh [out-dir]
#
# The out directory is `registry/` by default. It holds `index.json` and one
# tarball per package under `packages/`, which is exactly what a static host
# serves: the directory is the registry, and the URL of the directory is the
# registry's address.
#
# Publishing stays local. A package reaches a hosted registry by a pull
# request to its repository, reviewed by a person -- the same rule the
# registry's own README says.

set -eu

die() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"

out=${1:-"$root/registry"}
mkdir -p "$out/packages"

if [ -x target/release/yashy ]; then
    yashy="$root/target/release/yashy"
elif [ -x target/debug/yashy ]; then
    yashy="$root/target/debug/yashy"
else
    die "no yashy binary; run: cargo build --release -p yashy-driver"
fi

count=0
for package in "$root"/packages/*/; do
    [ -f "$package/yashy.toml" ] || continue
    name=$(basename -- "$package")
    printf 'publishing %s\n' "$name"
    "$yashy" publish "$package" --registry "$out" >/dev/null
    count=$((count + 1))
done

[ "$count" -gt 0 ] || die "no packages to publish"
printf 'published %d package(s) into %s\n' "$count" "$out"
