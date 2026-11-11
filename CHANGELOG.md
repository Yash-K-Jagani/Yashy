# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

The sections are generated from the conventional commits between two tags:

```text
scripts/changelog.sh --since v0.0.9 --version 0.1.0
```

A commit that changes nothing a user can see — `chore`, `ci`, `test` — is not
in the changelog. A commit that breaks a working program is listed first, under
**Breaking changes**.

## [Unreleased]

## [0.1.0] - 2026-11-11

### Added

- reproducible binaries from a fixed epoch and normalised paths ([8ff1644](https://github.com/Yash-K-Jagani/Yashy/commit/8ff1644))
- the macOS release -- signed, notarised, stapled, and an install test that checks Gatekeeper ([027388d](https://github.com/Yash-K-Jagani/Yashy/commit/027388d))
- fmt --check, --target and cfg, so CI can gate formatting and a program can be platform-specific (E2200-E2203) ([ed5d179](https://github.com/Yash-K-Jagani/Yashy/commit/ed5d179))
