# Installing Yashy

Yashy ships one signed archive per target. Every channel below downloads that
same archive and verifies its checksum or signature; a manifest that names a
different hash is a bug.

## Quick install

```text
# Linux and macOS
curl -fsSL https://github.com/Yash-K-Jagani/Yashy/releases/latest/download/install.sh | sh

# Windows PowerShell
iwr https://github.com/Yash-K-Jagani/Yashy/releases/latest/download/install.ps1 | iex
```

`yashyc doctor` checks the toolchain and stdlib discovery after the install.
`--uninstall` removes the installed files and the marked PATH block, leaving
the package cache untouched.

## Package managers

The repository carries manifests for the channels a release feeds:

| Channel | Source |
|---|---|
| WinGet | `scripts/install/winget/Yashy.Yashy.yaml` |
| Homebrew | `scripts/install/homebrew/yashy.rb` |
| apt | `scripts/install/apt/control` |
| Scoop | `scripts/install/scoop/yashy.json` |
| Docker | `scripts/install/Dockerfile` |

Each manifest pins a version and a SHA-256; the release workflow replaces the
`REPLACE_WITH_RELEASE_SHA256` placeholder before publishing.

## GitHub Action

```yaml
- uses: yashy/setup-yashy@v1
  with:
    version: v1.0.0
```

The action installs the pinned archive, adds `yashyc` to `PATH` and runs
`yashyc doctor`. It does not add a second compiler to the project.

## Self-update

```text
YASHY_TRUSTED_KEY=release:<hex-public-key> \
  yashyc self-update --archive yashy.tar.gz --signature yashy.tar.gz.sig
```

The signature is verified before any file is replaced. `--dry-run` verifies
and stops. On success the previous binary is kept as `yashyc.old` for rollback;
a failed replacement restores it.

The trust set is explicit: `YASHY_TRUSTED_KEY=id:hex_key` or a `YASHY_KEYRING`
file with `id=hex_key` lines. No key means no update.

## Supported targets

Windows x86-64 (MSVC), Linux x86-64 (GNU) and macOS x86-64 are the 1.0
targets; macOS is not a claimed platform until a signed, tested archive ships.
