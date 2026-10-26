# Yashy VERSION

SUMMARY: one paragraph about what changed. Keep it about the release, not the
whole project history.

## Targets

| Target | Archive | Minimum runtime |
|---|---|---|
| `x86_64-unknown-linux-gnu` | `yashy-VERSION-x86_64-unknown-linux-gnu.tar.gz` | glibc MINIMUM_GLIBC |
| `x86_64-pc-windows-msvc` | `yashy-VERSION-x86_64-pc-windows-msvc.zip` | Windows 10 or later |

macOS is a 1.0 target and is not part of this release. Do not claim macOS
support until a signed and tested archive ships.

## Requirements

- `clang` and LLVM. The compiler emits LLVM IR and asks `clang` to link it.
- Linux: the minimum glibc above.
- Windows: the Microsoft Visual C++ runtime that the MSVC toolchain links
  against.
- No Rust toolchain is required to install or use a release.

## Checksums

`SHA256SUMS` contains one line per archive. Verify it before installing:

```sh
sha256sum -c SHA256SUMS
```

On macOS, use `shasum -a 256 -c SHA256SUMS`. A checksum protects against a
truncated or corrupted download; it does not make a compromised release safe.
Compare the file through a second channel when that matters.

## Install

**Windows:**

```powershell
Expand-Archive yashy-VERSION-x86_64-pc-windows-msvc.zip -DestinationPath .
cd yashy-VERSION-x86_64-pc-windows-msvc
.\install.ps1
```

**Linux:**

```sh
tar xzf yashy-VERSION-x86_64-unknown-linux-gnu.tar.gz
cd yashy-VERSION-x86_64-unknown-linux-gnu
./install.sh
```

Or:

```sh
curl -fsSL https://github.com/Yash-K-Jagani/Otter/releases/download/vVERSION/install.sh | sh
```

Then:

```text
yashy doctor
yashy doctor --json
yashy init my-cli
```

The installer adds the prefix to `PATH` and can be removed with
`install.sh --uninstall` or `install.ps1 -Uninstall`. A second uninstall is a
no-op. `yashy uninstall` as a subcommand is not part of this release.

## Changes

- CHANGE
- CHANGE

## Known gaps

- macOS packaging and notarization are not in this release.
- GAP: any other gap that a user could reasonably hit.

## Verification

The release workflow builds both archives on their native hosts, installs the
Linux archive in a clean `ubuntu:22.04` container without Rust, runs
`yashy doctor`, `yashy doctor --json`, `yashy init`, and `yashy test`, then
uninstalls. The Windows archive goes through the same install, doctor, init,
and test path in CI before publishing.
