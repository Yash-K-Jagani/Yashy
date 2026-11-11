#!/usr/bin/env sh
# Stage a Unix release archive.
#
# The layout is the same on every platform:
#
#   yashy-<version>-<target>/
#     yashy
#     yashy-stdlib/yashy/*.ya
#     yashy-stdlib/yashy.toml
#     install.sh
#     README.md
#     LICENSE
#
# Usage:
#   scripts/package-release.sh --version <version> [--target <triple>] [--binary <path>]
#
# The default binary is target/release/yashy, which is what a native
# `cargo build --release -p yashy-driver` produces. --binary exists so a test
# or a cross build can name the artifact explicitly.

set -eu

die() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}

usage() {
    cat <<'EOF'
usage: scripts/package-release.sh --version <version> [--target <triple>] [--binary <path>] [--format tar|zip]

  --version <version>  release version, without a leading v (required)
  --target <triple>    target triple (default: x86_64-unknown-linux-gnu)
  --binary <path>      compiler binary (default: target/release/yashy)
  --format tar|zip     archive format (default: zip for an apple-darwin
                        target, tar for every other one)
EOF
}

version=""
target="x86_64-unknown-linux-gnu"
binary=""
format="auto"

while [ "$#" -gt 0 ]; do
    case "$1" in
        --version)
            [ "$#" -ge 2 ] || die "--version needs a value"
            version=$2
            shift 2
            ;;
        --target)
            [ "$#" -ge 2 ] || die "--target needs a value"
            target=$2
            shift 2
            ;;
        --binary)
            [ "$#" -ge 2 ] || die "--binary needs a value"
            binary=$2
            shift 2
            ;;
        --format)
            [ "$#" -ge 2 ] || die "--format needs a value"
            format=$2
            shift 2
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            die "unknown argument: $1"
            ;;
    esac
done

[ -n "$version" ] || die "--version is required"
case "$version" in
    v*) die "version must not start with v: $version" ;;
esac
case "$target" in
    */*|*\\*) die "target must be a triple, not a path: $target" ;;
esac

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
dist="$root/dist"
name="yashy-$version-$target"
stage="$dist/$name"
stdlib="$root/crates/yashy-stdlib/yashy"
stdlib_manifest="$root/crates/yashy-stdlib/yashy.toml"
installer="$root/scripts/install.sh"
readme="$root/README.md"
license="$root/LICENSE"

# The archive format follows the platform. A macOS release is a zip because
# Apple's notary accepts a zip, a dmg or a pkg and nothing else, and a
# notarised ticket can only be stapled to one of those. Every other platform
# keeps the tarball the installer and the checksum file already speak.
case "$format" in
    auto)
        case "$target" in
            *apple-darwin) format="zip" ;;
            *) format="tar" ;;
        esac
        ;;
esac
case "$format" in
    zip) archive="$dist/$name.zip" ;;
    tar) archive="$dist/$name.tar.gz" ;;
    *) die "unknown format: $format" ;;
esac

if [ -z "$binary" ]; then
    binary="$root/target/release/yashy"
fi

[ -f "$binary" ] || die "no release binary at $binary - run: cargo build --release -p yashy-driver"
[ -d "$stdlib" ] || die "no stdlib at $stdlib"
[ -f "$stdlib_manifest" ] || die "no stdlib manifest at $stdlib_manifest"
[ -f "$installer" ] || die "no installer at $installer"
[ -f "$readme" ] || die "no README at $readme"
[ -f "$license" ] || die "no LICENSE at $license"

rm -rf "$stage" "$archive"
mkdir -p "$stage/yashy-stdlib"

cp "$binary" "$stage/yashy"
chmod 755 "$stage/yashy"
cp -R "$stdlib" "$stage/yashy-stdlib/yashy"
cp "$stdlib_manifest" "$stage/yashy-stdlib/yashy.toml"
cp "$installer" "$stage/install.sh"
chmod 755 "$stage/install.sh"
cp "$readme" "$stage/README.md"
cp "$license" "$stage/LICENSE"

# -C makes the archive contain the top-level directory, which is the one
# layout both installers and a human extracting by hand can rely on.
case "$format" in
    zip)
        # -X keeps the extra file attributes out of the archive, so two builds
        # of the same tree produce the same bytes.
        (cd "$dist" && zip -q -r -X "$archive" "$name")
        ;;
    tar)
        tar -czf "$archive" -C "$dist" "$name"
        ;;
esac

printf '%s\n' "$archive"
