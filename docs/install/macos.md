# Installing Yashy on macOS

Yashy ships two macOS archives, one for Apple silicon and one for Intel:

```text
yashy-<version>-aarch64-apple-darwin.zip
yashy-<version>-x86_64-apple-darwin.zip
```

Both are built with a Developer ID Application certificate and submitted to
Apple's notary service once the release secrets have one, and then stapled, so
Gatekeeper accepts them without asking Apple's servers — including on a machine
with no network. While the Developer ID certificate is absent, the archives are
published unsigned, and the release notes say so.

## Which archive

| Mac | Archive |
|---|---|
| Apple silicon (M1, M2, M3, M4) | `yashy-<version>-aarch64-apple-darwin.zip` |
| Intel | `yashy-<version>-x86_64-apple-darwin.zip` |

`uname -m` answers `arm64` on Apple silicon and `x86_64` on Intel. A Mac
running the Rosetta translation answers `x86_64` and takes the Intel archive,
which runs on both.

## Install

The archive is a zip, because Apple's notary accepts a zip and nothing else.
Unzip it and run the installer it contains:

```text
unzip yashy-<version>-aarch64-apple-darwin.zip
sh yashy-<version>-aarch64-apple-darwin/install.sh --prefix "$HOME/.local"
```

The installer checks the archive against `SHA256SUMS`, copies the compiler and
the standard library into the prefix, adds the prefix to your shell profile,
and runs `yashy doctor` to tell you whether the toolchain is complete.

A file that came through a browser carries a quarantine attribute, and
Gatekeeper asks before it opens a program for the first time. A notarised
archive has nothing to ask about. An unsigned archive does: allow it under
**System Settings, Privacy & Security, Open Anyway**, or remove the attribute
yourself:

```text
xattr -dr com.apple.quarantine yashy-<version>-aarch64-apple-darwin
```

## What the compiler needs

Yashy compiles through LLVM's `clang`, so a macOS install needs the command
line tools:

```text
xcode-select --install
clang --version
```

`yashy doctor` reports the toolchain, the standard library and the PATH
entry, and `yashy doctor --json` prints the same facts as a document.

## Uninstall

```text
sh yashy-<version>-aarch64-apple-darwin/install.sh --uninstall --prefix "$HOME/.local"
```

The uninstall removes the installed files and the marked PATH block, and a
second run says there is nothing to remove.

## See also

- [The quickstart](../quickstart.md) — the first program.
- [The CLI reference](../cli.md) — every command and flag.
