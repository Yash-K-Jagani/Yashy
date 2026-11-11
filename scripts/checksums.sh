#!/usr/bin/env sh
# Write dist/SHA256SUMS for every release archive in dist.
#
# The release publish job downloads the Windows zip, the Linux tarball and the
# two macOS zips into one directory and runs this script once, so the
# checksum file describes every artifact. Running it in a single-platform job
# is fine too; it describes whatever archives are present.
#
# With --sign, the checksum file is also signed: `SHA256SUMS.asc` is a
# detached GPG signature, which is what lets a user check the hashes without
# trusting the download.
#
# Usage:
#   scripts/checksums.sh [--sign]
#
# Set YASHY_DIST to checksum a different directory (used by the installer
# tests). Set YASHY_GPG_KEY to the key id to sign with; the default is the
# key the environment names.

set -eu

die() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
dist=${YASHY_DIST:-"$root/dist"}
sign=0

while [ "$#" -gt 0 ]; do
    case "$1" in
        --sign)
            sign=1
            shift
            ;;
        -h|--help)
            printf '%s\n' "usage: scripts/checksums.sh [--sign]"
            exit 0
            ;;
        *)
            die "unknown argument: $1"
            ;;
    esac
done

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

if [ "$sign" -eq 1 ]; then
    # A detached signature, so a user can check the hashes without trusting
    # the download. The key is the one the environment names; a job that has
    # no key must fail loudly rather than publish an unsigned file.
    command -v gpg >/dev/null 2>&1 || die "no gpg found; install it to sign the checksum file"
    # A passphrase-protected key needs the loopback pinentry in a job: there
    # is no terminal to ask.
    if [ -n "${YASHY_GPG_PASSPHRASE:-}" ]; then
        passphrase_args="--pinentry-mode loopback --passphrase $YASHY_GPG_PASSPHRASE"
    else
        passphrase_args=""
    fi
    # shellcheck disable=SC2086
    if [ -n "${YASHY_GPG_KEY:-}" ]; then
        gpg --batch --yes $passphrase_args --local-user "$YASHY_GPG_KEY" \
            --armor --detach-sign --output "$out.asc" "$out"
    else
        gpg --batch --yes $passphrase_args --armor --detach-sign --output "$out.asc" "$out"
    fi
    printf '%s\n' "$out.asc"
fi

printf '%s\n' "$out"
