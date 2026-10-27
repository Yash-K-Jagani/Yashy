#!/usr/bin/env sh
# Exercise scripts/install.sh against a local release archive.
#
# This is the Linux half of the clean-install test. It uses a temporary HOME
# and prefix, so it never edits the caller's shell profiles and never needs
# root. It does need clang/LLVM for the doctor-success cases; the
# doctor-failure case installs a fake clang that refuses to run.
#
# Usage:
#   sh scripts/test-install.sh [--archive <tarball>]
#
# Without --archive, the script takes the single *.tar.gz in dist/.

set -eu

die() {
    printf 'FAIL: %s\n' "$*" >&2
    exit 1
}

ok() {
    printf 'ok - %s\n' "$*"
}

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
archive=""

while [ "$#" -gt 0 ]; do
    case "$1" in
        --archive)
            [ "$#" -ge 2 ] || die "--archive needs a value"
            archive=$2
            shift 2
            ;;
        -h|--help)
            printf '%s\n' "usage: sh scripts/test-install.sh [--archive <tarball>]"
            exit 0
            ;;
        *)
            die "unknown argument: $1"
            ;;
    esac
done

if [ -z "$archive" ]; then
    for candidate in "$root"/dist/yashy-*-x86_64-unknown-linux-gnu.tar.gz; do
        [ -f "$candidate" ] || continue
        [ -z "$archive" ] || die "more than one Linux archive in dist; pass --archive"
        archive=$candidate
    done
fi
[ -n "$archive" ] || die "no Linux archive; build one with scripts/package-release.sh or pass --archive"
[ -f "$archive" ] || die "no archive at $archive"

tmp=$(mktemp -d "${TMPDIR:-/tmp}/yashy-test-install.XXXXXX")
cleanup() {
    rm -rf "$tmp"
}
trap cleanup 0 1 2 15

home="$tmp/home"
work="$tmp/work"
mkdir -p "$home" "$work"
cp "$archive" "$work/"
archive_name=$(basename -- "$archive")
archive="$work/$archive_name"
YASHY_DIST="$work" sh "$root/scripts/checksums.sh" >/dev/null
checksums="$work/SHA256SUMS"

# Run the installer with an isolated HOME and the local archive.
run_install() {
    prefix=$1
    shift
    HOME="$home" \
        YASHY_ARCHIVE="$archive" \
        YASHY_SHA256SUMS="$checksums" \
        sh "$root/scripts/install.sh" --prefix "$prefix" "$@"
}

# 1. A fresh prefix installs, doctor passes, and the first project runs.
prefix="$tmp/prefix-fresh"
output=$(run_install "$prefix" --no-path)
printf '%s\n' "$output" | grep -q "Installed Yashy" || die "install did not report success"
[ -x "$prefix/bin/yashy" ] || die "the installer did not create bin/yashy"
[ -f "$prefix/share/yashy/stdlib/yashy/io.ya" ] ||
    die "the installer did not copy the stdlib to the nested shared-prefix layout"
PATH="$prefix/bin:$PATH" "$prefix/bin/yashy" doctor >/dev/null || die "doctor failed after a clean install"
json=$(PATH="$prefix/bin:$PATH" "$prefix/bin/yashy" doctor --json) || die "doctor --json failed after a clean install"
printf '%s\\n' "$json" | grep -q '"schema_version": 1' || die "doctor --json has no schema_version"
printf '%s\\n' "$json" | grep -q '"tool": "doctor"' || die "doctor --json has no tool"
printf '%s\\n' "$json" | grep -q '"ok": true' || die "doctor --json did not report ok"
printf '%s\\n' "$json" | grep -q '"next": null' || die "doctor --json has a next step on success"
demo="$tmp/demo"
"$prefix/bin/yashy" init "$demo" >/dev/null || die "init failed"
(cd "$demo" && "$prefix/bin/yashy" run main.ya >/dev/null) || die "run failed"
(cd "$demo" && "$prefix/bin/yashy" test >/dev/null) || die "test failed"
ok "fresh install, doctor, init, run, and test"

# 2. Re-running is harmless.
output=$(run_install "$prefix" --no-path)
printf '%s\n' "$output" | grep -q "Installed Yashy" || die "reinstall did not report success"
ok "rerun is idempotent"

# 3. The PATH block is added once and removed by uninstall.
home_path="$tmp/home-path"
prefix_path="$tmp/prefix-path"
mkdir -p "$home_path"
HOME="$home_path" YASHY_ARCHIVE="$archive" YASHY_SHA256SUMS="$checksums" \
    sh "$root/scripts/install.sh" --prefix "$prefix_path" --no-doctor >/dev/null
[ -f "$home_path/.profile" ] || die "the installer did not create .profile"
count=$(grep -c "# >>> yashy installer >>>" "$home_path/.profile" || true)
[ "$count" -eq 1 ] || die "expected one PATH block in .profile, found $count"
grep -q "$prefix_path/bin" "$home_path/.profile" || die "the PATH block does not name the prefix"
HOME="$home_path" YASHY_ARCHIVE="$archive" YASHY_SHA256SUMS="$checksums" \
    sh "$root/scripts/install.sh" --prefix "$prefix_path" --no-doctor >/dev/null
count=$(grep -c "# >>> yashy installer >>>" "$home_path/.profile" || true)
[ "$count" -eq 1 ] || die "a second install duplicated the PATH block"
ok "PATH block is added once"

# 4. --no-path leaves profiles alone.
home_nopath="$tmp/home-nopath"
prefix_nopath="$tmp/prefix-nopath"
mkdir -p "$home_nopath"
HOME="$home_nopath" YASHY_ARCHIVE="$archive" YASHY_SHA256SUMS="$checksums" \
    sh "$root/scripts/install.sh" --prefix "$prefix_nopath" --no-path --no-doctor >/dev/null
[ ! -e "$home_nopath/.profile" ] || die "--no-path still created .profile"
[ ! -e "$home_nopath/.bashrc" ] || die "--no-path still created .bashrc"
ok "--no-path leaves shell profiles alone"

# 5. --no-doctor does not invoke doctor.
home_nodoc="$tmp/home-nodoc"
prefix_nodoc="$tmp/prefix-nodoc"
mkdir -p "$home_nodoc"
output=$(HOME="$home_nodoc" YASHY_ARCHIVE="$archive" YASHY_SHA256SUMS="$checksums" \
    sh "$root/scripts/install.sh" --prefix "$prefix_nodoc" --no-path --no-doctor)
printf '%s\n' "$output" | grep -q "Installed Yashy" || die "--no-doctor did not install"
if printf '%s\n' "$output" | grep -q "clang:"; then
    die "--no-doctor still ran doctor"
fi
ok "--no-doctor skips doctor"

# 6. A missing clang keeps the install and prints the distro command.
fakebin="$tmp/fakebin"
prefix_noclang="$tmp/prefix-noclang"
mkdir -p "$fakebin"
printf '#!/bin/sh\nexit 1\n' > "$fakebin/clang"
chmod +x "$fakebin/clang"
output=$(PATH="$fakebin:$PATH" HOME="$home" YASHY_ARCHIVE="$archive" \
    YASHY_SHA256SUMS="$checksums" \
    sh "$root/scripts/install.sh" --prefix "$prefix_noclang" --no-path 2>&1) ||
    die "the installer must succeed when doctor fails"
printf '%s\n' "$output" | grep -q "Yashy installed, but" || die "missing-clang message is absent"
printf '%s\n' "$output" | grep -Eq "Next: (sudo apt install clang llvm|sudo dnf install clang llvm|sudo pacman -S clang llvm|install clang and LLVM)" ||
    die "missing-clang next step is not actionable"
[ -x "$prefix_noclang/bin/yashy" ] || die "a missing compiler must not roll back the install"
ok "missing clang keeps the install and prints an actionable next step"

# 7. A bad checksum is a hard failure and leaves no partial install.
badwork="$tmp/bad"
prefix_bad="$tmp/prefix-bad"
mkdir -p "$badwork"
cp "$archive" "$badwork/$archive_name"
printf '%064d  %s\n' 0 "$archive_name" > "$badwork/SHA256SUMS"
if HOME="$home" YASHY_ARCHIVE="$badwork/$archive_name" \
    YASHY_SHA256SUMS="$badwork/SHA256SUMS" \
    sh "$root/scripts/install.sh" --prefix "$prefix_bad" --no-path --no-doctor \
    >"$tmp/bad.out" 2>&1; then
    die "a bad checksum must fail the install"
fi
grep -q "checksum mismatch" "$tmp/bad.out" || die "the bad-checksum error is not clear"
[ ! -e "$prefix_bad/bin/yashy" ] || die "a bad checksum left a partial install"
ok "bad checksum fails before writing the prefix"

# 8. Uninstall removes the prefix and the PATH block, and a second run is a no-op.
HOME="$home_path" sh "$root/scripts/install.sh" --uninstall --prefix "$prefix_path" --no-path >/dev/null
[ ! -d "$prefix_path" ] || die "uninstall left the prefix behind"
if grep -q "# >>> yashy installer >>>" "$home_path/.profile"; then
    die "uninstall left the PATH block behind"
fi
output=$(HOME="$home_path" sh "$root/scripts/install.sh" --uninstall --prefix "$prefix_path" --no-path)
printf '%s\n' "$output" | grep -q "nothing to remove" || die "a second uninstall is not a no-op"
ok "uninstall removes files and the PATH block, and is idempotent"

# 9. An extracted archive installs without a checksum file.
extract="$tmp/extract"
prefix_extract="$tmp/prefix-extract"
mkdir -p "$extract"
tar -xzf "$archive" -C "$extract"
stage=""
for candidate in "$extract"/*; do
    if [ -d "$candidate" ] && [ -f "$candidate/yashy" ]; then
        stage=$candidate
        break
    fi
done
[ -n "$stage" ] || die "the archive has no top-level package directory"
HOME="$home" sh "$stage/install.sh" --prefix "$prefix_extract" --no-path --no-doctor >/dev/null
[ -x "$prefix_extract/bin/yashy" ] || die "the extracted installer did not install"
ok "extracted archive installs without a download"

printf '\nall installer tests passed\n'
