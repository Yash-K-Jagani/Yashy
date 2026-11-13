# Releasing Yashy

A release is a tag. The tag starts a workflow that builds every platform, signs
what it can, runs the install tests, and publishes the release with the
changelog. Nothing is published from a working tree: the only way to publish is
to tag a commit, so the artifacts and the source that produced them always
share one tree.

Publishing is the one act a release cannot undo, so the workflow decides what
the release *is* before it publishes, and says so in the notes. A missing
signature is an unsigned build, not a failure and not a silent one: the notes
open with a section naming every signature the download does not carry.

## What a release ships

| Platform | Artifact | Signed how |
|---|---|---|
| Linux x86-64 | `yashy-<version>-x86_64-unknown-linux-gnu.tar.gz` | the checksum file is GPG-signed: `SHA256SUMS.asc` |
| macOS (Apple silicon) | `yashy-<version>-aarch64-apple-darwin.zip` | Developer ID Application certificate and notarised; Gatekeeper fetches the ticket on the first run |
| macOS (Intel) | `yashy-<version>-x86_64-apple-darwin.zip` | Developer ID Application certificate and notarised; Gatekeeper fetches the ticket on the first run |
| Windows x86-64 | `yashy-<version>-x86_64-pc-windows-msvc.zip` | Authenticode on the compiler binary, with a timestamping authority |

Every release also ships `SHA256SUMS`, one line per archive, and the two
installers (`install.sh`, `install.ps1`).

A macOS release is a zip because Apple's notary accepts a zip, a dmg or a pkg
and nothing else. The ticket cannot be stapled to a zip or to a bare
executable — `stapler` supports an app bundle, a disk image and a flat
installer package — so the notary's verdict is published against the binary and
Gatekeeper fetches it on the first run. A first run therefore needs a network
connection; a `.dmg` or `.pkg` is what would make it work offline. The Linux
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

A missing secret never fails a tag and never produces an artifact that claims a
signature it does not have. The job continues with an `::warning::` naming the
secret, and the publish job prepends "This build is unsigned" to the notes with
one bullet per skipped signature: SmartScreen for Windows, the Gatekeeper
override for macOS, the absent `SHA256SUMS.asc` for the checksums. The install
test does the same: it probes the macOS archive with `codesign --verify` and
runs the full Gatekeeper assertions only when a signature is there.

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

The release workflow writes the same section into the release notes, so the
page and the file cannot disagree.

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
   the secrets allow, notarises the macOS archives when it can, runs the
   install tests, and publishes the release with the changelog. The notes say
   which signatures the build carries.

4. **Read the published notes.** If the build is unsigned, the notes open with
   "This build is unsigned" and the exact one-time override a user needs per
   platform. If a signature is missing that should be there, add the secret
   and re-tag; do not edit the release by hand.

5. **Watch the install matrix.** It runs on every published release and
   weekly, installing the published artifacts on Linux, Windows and macOS. A
   release that breaks an installer is caught there rather than by a user.

## A dry release

A `workflow_dispatch` of `release.yml` with `dry` ticked is the rehearsal: it
builds every archive, skips the signatures it cannot make, runs the Linux
`package-smoke`, installs the macOS archive with `--unsigned`, and uploads one
`yashy-dry-release` artifact. It creates no release and touches no tag.

Tagging a release candidate (`v0.1.0-rc1`) also publishes, so a tag is not a
rehearsal any more. Use `dry` for that; a pre-release tag is a public release
that happens to be a candidate, and GitHub shows it as such only if
`prerelease` is set, which the workflow does not set.

The install matrix installs a dry bundle on all three platforms: run
`install-matrix.yml` by hand with `mode: run` and the release run's id. It
detects an unsigned archive on its own and skips the signature, notarisation
and Gatekeeper assertions with a warning, so the run is meaningful before the
certificates exist instead of permanently red.

## Reproducible builds

`yashy build --reproducible` fixes the timestamp, normalises the source paths
and asks the linker for a reproducible image, so two builds of the same tree
produce the same bytes. The release archives are built with it, so the
checksum a user reads is the checksum of the tree and not of a moment.

## See also

- [Installing on macOS](install/macos.md)
- [The CLI reference](cli.md) — `--reproducible` and `--target`.
- [The error catalog](errors.md)
