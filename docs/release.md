# Releasing Yashy

A release is a tag. The tag starts a workflow that builds every platform,
signs what it can, and writes a **draft** release; a person then reads the
notes and publishes it. Nothing is published automatically, because the one
thing a release cannot undo is a broken artifact that thousands of people
install.

## What a release ships

| Platform | Artifact | Signed how |
|---|---|---|
| Linux x86-64 | `yashy-<version>-x86_64-unknown-linux-gnu.tar.gz` | the checksum file is GPG-signed: `SHA256SUMS.asc` |
| macOS (Apple silicon) | `yashy-<version>-aarch64-apple-darwin.zip` | Developer ID Application certificate, notarised and stapled |
| macOS (Intel) | `yashy-<version>-x86_64-apple-darwin.zip` | Developer ID Application certificate, notarised and stapled |
| Windows x86-64 | `yashy-<version>-x86_64-pc-windows-msvc.zip` | Authenticode on the compiler binary, with a timestamping authority |

Every release also ships `SHA256SUMS`, one line per archive, and the two
installers (`install.sh`, `install.ps1`).

A macOS release is a zip because Apple's notary accepts a zip, a dmg or a pkg
and nothing else, and a ticket can only be stapled to one of those. The Linux
release stays a tarball.

## The secrets

| Secret | Used for |
|---|---|
| `MACOS_CERT_P12_BASE64`, `MACOS_CERT_PASSWORD` | the Developer ID certificate |
| `MACOS_NOTARY_KEY_BASE64`, `MACOS_NOTARY_KEY_ID`, `MACOS_NOTARY_ISSUER` | the App Store Connect API key `notarytool` submits with (the issuer is needed for a team key) |
| `MACOS_APPLE_ID`, `MACOS_APPLE_PASSWORD`, `MACOS_TEAM_ID` | the alternative: an Apple ID with an app-specific password, stored as a `notarytool` keychain profile |
| `WINDOWS_CERT_PFX_BASE64`, `WINDOWS_CERT_PASSWORD` | the Authenticode certificate |
| `RELEASE_GPG_PRIVATE_KEY`, `RELEASE_GPG_PASSPHRASE` | the signature over `SHA256SUMS` |

The notary takes either the API key or the Apple ID; the API key is the one
that works unattended in CI, so it is tried first. Every value is base64 of
the file or text, because a workflow secret is a string.

A job whose secret is missing fails with the name of the secret. It never
produces an unsigned artifact that looks like a release. The one exception is
a `dry` dispatch, which says in the log which signature it skipped and
publishes nothing.

### Creating the GPG key

The checksum signature is only worth something if the key is the release
manager's, so generate it locally and upload only the private key to the
repository's secrets:

```text
gpg --quick-generate-key 'Yashy Release <releases@example.org>' rsa4096 sign 2y
gpg --armor --export 'Yashy Release' > release.asc          # publish this
gpg --armor --export-secret-keys 'Yashy Release' \
  | base64 > release-key.b64                                 # keep this out of git
```

Then set `RELEASE_GPG_PRIVATE_KEY` to the contents of `release-key.b64` and
`RELEASE_GPG_PASSPHRASE` to its passphrase, delete the file, and publish
`release.asc` beside the release so a user can import it before running
`gpg --verify SHA256SUMS.asc SHA256SUMS`.

## The changelog

`CHANGELOG.md` follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Its sections are generated from the conventional commits between two tags:

```text
scripts/changelog.sh --since v0.0.9 --version 0.1.0
```

The script groups the commits by their type: `feat` under **Added**, `fix`
under **Fixed**, and `perf`, `refactor`, `docs` and `build` under **Changed**.
A commit that breaks a working program is listed first, under **Breaking
changes**. A commit that changes nothing a user can see — `chore`, `ci`,
`test` — is not in the changelog at all.

The release workflow writes the same section into the draft release's notes, so
the page and the file cannot disagree.

## Making a release

1. **Check the tree.** `cargo test --workspace --no-fail-fast`, and
   `scripts/repro-check.sh` (or `scripts/repro-check.ps1`), which builds the
   same program twice in two directories and compares the SHA-256 of the two
   binaries. Two builds that differ mean the release cannot be recomputed by
   a stranger, and that is a bug worth fixing before the tag, not after.

2. **Tag it.** A `v`-prefixed tag, for example `v0.1.0`:

   ```text
   git tag -a v0.1.0 -m "Yashy 0.1.0"
   git push origin v0.1.0
   ```

3. **Wait for the workflow.** It builds Linux, Windows and macOS, signs what
   it can, notarises the macOS archives, runs the install tests, and writes a
   draft release with the changelog.

4. **Read the draft and publish it.** The draft is not the release until a
   person publishes it.

5. **Watch the install matrix.** It runs on every published release and
   weekly, installing the published artifacts on Linux, Windows and macOS. A
   release that breaks an installer is caught there rather than by a user.

## A dry release

A pre-release tag exercises the whole path without publishing anything:

```text
git tag -a v0.1.0-rc1 -m "Yashy 0.1.0 release candidate"
git push origin v0.1.0-rc1
```

The workflow behaves the same way and writes the same draft; the difference is
the tag, and the draft's title. A dry release is the only way to see the
artifacts before they are public, so do one before every first release of a
kind -- the first macOS archive, the first signed checksum file.

Before there are credentials, the same path can be run without them. A
`workflow_dispatch` of `release.yml` with `dry` ticked builds every archive,
skips the signatures it cannot make, runs the Linux `package-smoke`, installs
the macOS archive with `--unsigned`, and uploads one `yashy-dry-release`
artifact. It creates no release and touches no tag.

The install matrix then installs that bundle on all three platforms: run
`install-matrix.yml` by hand with `mode: run`, the release run's id, and
`unsigned` ticked. That executes the same install checks a published release
gets, minus the signature, notarisation and Gatekeeper assertions -- which are
exactly what the missing credentials would let it assert.

## Reproducible builds

`yashy build --reproducible` fixes the timestamp, normalises the source paths
and asks the linker for a reproducible image, so two builds of the same tree
produce the same bytes. The release archives are built with it, so the
checksum a user reads is the checksum of the tree and not of a moment.

## See also

- [Installing on macOS](install/macos.md)
- [The CLI reference](cli.md) — `--reproducible` and `--target`.
- [The error catalog](errors.md)
