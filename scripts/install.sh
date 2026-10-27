#!/usr/bin/env sh
# Install a prebuilt Yashy compiler on Linux x86-64.
#
#   curl -fsSL https://github.com/Yash-K-Jagani/Otter/releases/latest/download/install.sh | sh
#   sh install.sh --prefix "$HOME/.local"
#   sh install.sh --uninstall
#
# The script is POSIX sh, not bash, because the quickstart advertises
# `curl ... | sh`. It installs the compiler at $PREFIX/bin/yashy and the
# bundled standard library at $PREFIX/share/yashy/stdlib/yashy, so the
# compiler finds the library without YASHY_PATH.
#
# Environment:
#   YASHY_VERSION       release version (default: latest)
#   YASHY_PREFIX        install prefix (default: $HOME/.local)
#   YASHY_ARCHIVE       local tarball, for offline installs and tests
#   YASHY_SHA256SUMS    checksum file for YASHY_ARCHIVE (default: beside it)
#
# Flags:
#   --version <v>       release version
#   --prefix <dir>      install prefix
#   --no-path           do not edit shell profiles
#   --no-doctor         do not run `yashy doctor` after installing
#   --uninstall         remove the installed files and PATH block
#   --help              show this help

set -eu

REPO="Yash-K-Jagani/Otter"
TARGET="x86_64-unknown-linux-gnu"

die() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}

say() {
    printf '%s\n' "$*"
}

usage() {
    cat <<'EOF'
usage: install.sh [options]

  --version <v>   install a specific release (default: latest)
  --prefix <dir>  install prefix (default: $HOME/.local)
  --no-path       do not edit ~/.profile, ~/.bashrc, or ~/.zshrc
  --no-doctor     do not run `yashy doctor` after installing
  --uninstall     remove the installed files and the marked PATH block
  --help          show this help

environment:
  YASHY_VERSION, YASHY_PREFIX, YASHY_ARCHIVE, YASHY_SHA256SUMS
EOF
}

version=${YASHY_VERSION:-latest}
prefix=${YASHY_PREFIX:-"${HOME:-}/.local"}
do_path=1
do_doctor=1
uninstall=0
archive=${YASHY_ARCHIVE:-}
checksums=${YASHY_SHA256SUMS:-}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --version)
            [ "$#" -ge 2 ] || die "--version needs a value"
            version=$2
            shift 2
            ;;
        --prefix)
            [ "$#" -ge 2 ] || die "--prefix needs a value"
            prefix=$2
            shift 2
            ;;
        --no-path)
            do_path=0
            shift
            ;;
        --no-doctor)
            do_doctor=0
            shift
            ;;
        --uninstall)
            uninstall=1
            shift
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

[ -n "${HOME:-}" ] || die "HOME is not set"
[ -n "$prefix" ] || die "the install prefix is empty"
case "$prefix" in
    /) die "refusing to use / as the install prefix" ;;
    *'
'*) die "the install prefix must not contain a newline" ;;
esac
# A trailing slash would make the PATH entry look different on every run.
prefix=${prefix%/}
[ -n "$prefix" ] || die "the install prefix is empty"

sha256_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | awk '{print $1}'
    else
        die "no SHA-256 tool found; install sha256sum (coreutils) or shasum"
    fi
}

llvm_hint() {
    if [ -r /etc/os-release ]; then
        # The file is the distribution's own metadata; reading it is the
        # standard way to tell apt from dnf from pacman without guessing.
        # shellcheck disable=SC1091
        . /etc/os-release
        case "${ID:-} ${ID_LIKE:-}" in
            *ubuntu*|*debian*) printf '%s\n' "sudo apt install clang llvm" ;;
            *fedora*|*rhel*|*centos*) printf '%s\n' "sudo dnf install clang llvm" ;;
            *arch*) printf '%s\n' "sudo pacman -S clang llvm" ;;
            *) printf '%s\n' "install clang and LLVM with your package manager" ;;
        esac
    else
        printf '%s\n' "install clang and LLVM with your package manager"
    fi
}

remove_path_block() {
    file=$1
    [ -f "$file" ] || return 0
    tmp="$file.yashy-tmp.$$"
    awk '
        $0 == "# >>> yashy installer >>>" { skip = 1; next }
        $0 == "# <<< yashy installer <<<" { skip = 0; next }
        skip != 1 { print }
    ' "$file" > "$tmp"
    if cmp -s "$file" "$tmp"; then
        rm -f "$tmp"
    else
        mv "$tmp" "$file"
        say "removed the Yashy PATH block from $file"
    fi
}

add_path_block() {
    file=$1
    [ -f "$file" ] || : > "$file"
    tmp="$file.yashy-tmp.$$"

    # Escape the prefix for a double-quoted shell string. A prefix with a
    # newline was rejected above; these four characters are the rest of the
    # shell's double-quote syntax.
    escaped=$(printf '%s' "$prefix/bin" |
        sed -e 's/\\/\\\\/g' -e 's/"/\\"/g' -e 's/\$/\\$/g' -e 's/`/\\`/g')

    awk '
        $0 == "# >>> yashy installer >>>" { skip = 1; next }
        $0 == "# <<< yashy installer <<<" { skip = 0; next }
        skip != 1 { print }
    ' "$file" > "$tmp"

    {
        printf '%s\n' "# >>> yashy installer >>>"
        printf '%s\n' "# Added by the Yashy installer for $prefix"
        printf '%s\n' 'case ":$PATH:" in'
        printf '%s\n' "  *\":$escaped:\"*) ;;"
        printf '%s\n' "  *) export PATH=\"$escaped:\$PATH\" ;;"
        printf '%s\n' 'esac'
        printf '%s\n' "# <<< yashy installer <<<"
    } >> "$tmp"

    if cmp -s "$file" "$tmp"; then
        rm -f "$tmp"
        say "PATH already contains $prefix/bin in $file"
    else
        mv "$tmp" "$file"
        say "added $prefix/bin to PATH in $file"
    fi
}

if [ "$uninstall" -eq 1 ]; then
    removed=0

    remove_path_block "$HOME/.profile"
    remove_path_block "$HOME/.bashrc"
    if [ -f "$HOME/.zshrc" ]; then
        remove_path_block "$HOME/.zshrc"
    fi

    if [ -f "$prefix/bin/yashy" ]; then
        rm -f "$prefix/bin/yashy"
        say "removed $prefix/bin/yashy"
        removed=1
    fi
    if [ -d "$prefix/share/yashy" ]; then
        rm -rf "$prefix/share/yashy"
        say "removed $prefix/share/yashy"
        removed=1
    fi
    # Remove only directories that are now empty. A prefix such as
    # $HOME/.local is shared with other programs and must survive.
    rmdir "$prefix/bin" 2>/dev/null || true
    rmdir "$prefix/share" 2>/dev/null || true
    rmdir "$prefix" 2>/dev/null || true

    if [ "$removed" -eq 0 ]; then
        say "nothing to remove at $prefix"
    fi
    say "Yashy uninstalled from $prefix"
    exit 0
fi

uname_s=$(uname -s 2>/dev/null || true)
uname_m=$(uname -m 2>/dev/null || true)
if [ "$uname_s" != "Linux" ] || [ "$uname_m" != "x86_64" ]; then
    die "this release supports Linux x86-64 only; detected ${uname_s:-unknown} ${uname_m:-unknown}. macOS is a 1.0 target."
fi

tmp=""
cleanup() {
    if [ -n "$tmp" ] && [ -d "$tmp" ]; then
        rm -rf "$tmp"
    fi
}
trap cleanup 0 1 2 15

script_dir=""
if [ -f "$0" ]; then
    script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
fi

if [ -n "$archive" ]; then
    [ -f "$archive" ] || die "no archive at $archive"
    if [ -z "$checksums" ]; then
        checksums=$(dirname -- "$archive")/SHA256SUMS
    fi
    [ -f "$checksums" ] || die "no checksum file at $checksums; set YASHY_SHA256SUMS"
    archive_name=$(basename -- "$archive")
    expected=$(tr -d '\r' < "$checksums" |
        awk -v name="$archive_name" '$2 == name { print $1; exit }' |
        tr 'A-F' 'a-f')
    [ -n "$expected" ] || die "no checksum for $archive_name in $checksums"
    actual=$(sha256_file "$archive")
    [ "$expected" = "$actual" ] ||
        die "checksum mismatch for $archive_name: expected $expected, got $actual"
    source_mode="archive"
elif [ -n "$script_dir" ] && [ -f "$script_dir/yashy" ] &&
    [ -d "$script_dir/yashy-stdlib/yashy" ]; then
    # The script is inside an extracted release archive. There is no download
    # and no archive to checksum; the user already has the files.
    source_mode="extracted"
    archive=""
else
    source_mode="download"
fi

if [ "$source_mode" = "download" ]; then
    tmp=$(mktemp -d "${TMPDIR:-/tmp}/yashy-install.XXXXXX")

    if [ "$version" = "latest" ]; then
        api="https://api.github.com/repos/$REPO/releases/latest"
        release_json=$(curl -fsSL "$api") ||
            die "cannot reach the GitHub release API at $api"
        version=$(printf '%s\n' "$release_json" |
            sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"v\{0,1\}\([^"]*\)".*/\1/p' |
            head -n 1)
        [ -n "$version" ] || die "the latest release has no readable tag name"
    fi

    archive="$tmp/yashy-$version-$TARGET.tar.gz"
    checksums="$tmp/SHA256SUMS"
    base_url="https://github.com/$REPO/releases/download/v$version"
    curl -fsSL "$base_url/yashy-$version-$TARGET.tar.gz" -o "$archive" ||
        die "cannot download $base_url/yashy-$version-$TARGET.tar.gz"
    curl -fsSL "$base_url/SHA256SUMS" -o "$checksums" ||
        die "cannot download $base_url/SHA256SUMS"
    expected=$(tr -d '\r' < "$checksums" |
        awk -v name="yashy-$version-$TARGET.tar.gz" '$2 == name { print $1; exit }' |
        tr 'A-F' 'a-f')
    [ -n "$expected" ] || die "SHA256SUMS has no entry for yashy-$version-$TARGET.tar.gz"
    actual=$(sha256_file "$archive")
    [ "$expected" = "$actual" ] ||
        die "checksum mismatch for yashy-$version-$TARGET.tar.gz: expected $expected, got $actual"
fi

if [ "$source_mode" = "archive" ] || [ "$source_mode" = "download" ]; then
    [ -n "$tmp" ] || tmp=$(mktemp -d "${TMPDIR:-/tmp}/yashy-install.XXXXXX")
    extract="$tmp/extract"
    mkdir -p "$extract"
    tar -xzf "$archive" -C "$extract" || die "cannot extract $archive"
    stage=""
    for candidate in "$extract"/*; do
        if [ -d "$candidate" ] && [ -f "$candidate/yashy" ]; then
            stage=$candidate
            break
        fi
    done
    [ -n "$stage" ] || die "the archive does not contain a yashy binary"
    [ -d "$stage/yashy-stdlib/yashy" ] || die "the archive does not contain yashy-stdlib/yashy"
else
    stage=$script_dir
fi

mkdir -p "$prefix/bin" "$prefix/share/yashy/stdlib"
cp "$stage/yashy" "$prefix/bin/yashy"
chmod 755 "$prefix/bin/yashy"
rm -rf "$prefix/share/yashy/stdlib"
mkdir -p "$prefix/share/yashy/stdlib"
cp -R "$stage/yashy-stdlib/yashy" "$prefix/share/yashy/stdlib/yashy"

if [ "$do_path" -eq 1 ]; then
    add_path_block "$HOME/.profile"
    add_path_block "$HOME/.bashrc"
    if [ -f "$HOME/.zshrc" ]; then
        add_path_block "$HOME/.zshrc"
    fi
else
    say "skipped PATH setup (--no-path)"
fi

say "Installed Yashy to $prefix"

if [ "$do_doctor" -eq 1 ]; then
    doctor_status=0
    # Put the prefix on PATH for this one call so doctor's PATH check sees the
    # install that was just made, even before the user opens a new shell.
    if ! PATH="$prefix/bin:$PATH" "$prefix/bin/yashy" doctor; then
        doctor_status=1
    fi
    if [ "$doctor_status" -ne 0 ]; then
        say ""
        say "Yashy installed, but \`yashy doctor\` found a problem."
        say "Next: $(llvm_hint)"
        say "Then run: yashy doctor"
        exit 0
    fi
fi

say "Next: yashy init my-cli"
if [ "$do_path" -eq 0 ]; then
    say "      (add $prefix/bin to PATH, or run $prefix/bin/yashy)"
fi
