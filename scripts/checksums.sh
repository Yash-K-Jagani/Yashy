#!/usr/bin/env sh
# Write dist/SHA256SUMS for every release archive in dist.
#
# The release publish job downloads the Windows zip and the Linux tarball into
# one directory and runs this script once, so the checksum file describes both
# artifacts. Running it in a single-platform job is fine too; it describes
# whatever archives are present.
#
# Usage:
#   scripts/checksums.sh
#
# Set YASHY_DIST to checksum a different directory (used by the installer
# tests).

set -eu

die() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
dist=${YASHY_DIST:-"$root/dist"}

[ -d "$dist" ] || die "no dist directory at $dist"

hash_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | awk '{print $1}'
    else
        die "no SHA-256 tool found; install sha256sum (coreutils) or shasum"
    fi
}

out="$dist/SHA256SUMS"
tmp="$out.tmp"
: > "$tmp"

found=0
while IFS= read -r archive; do
    [ -n "$archive" ] || continue
    found=1
    base=$(basename -- "$archive")
    hash=$(hash_file "$archive")
    printf '%s  %s\n' "$hash" "$base" >> "$tmp"
done <<EOF
$(find "$dist" -maxdepth 1 -type f \( -name '*.tar.gz' -o -name '*.zip' \) | sort)
EOF

if [ "$found" -eq 0 ]; then
    rm -f "$tmp"
    die "no release archives (*.tar.gz, *.zip) in $dist"
fi

mv "$tmp" "$out"
printf '%s\n' "$out"
