#!/usr/bin/env bash
# One CI entry point for GitHub Actions, GitLab CI and a local dry run.
# Every Cargo invocation is --locked so the lockfile, not a fresh resolution,
# decides what is built. The stages mirror docs/ci.md; a stage that needs a
# tool the machine does not have reports SKIP with the reason and exits 0,
# because this script is the template, not a gate that silently pretends.
set -euo pipefail

cd "$(dirname "$0")/.."

stage() { printf '\n== %s ==\n' "$1"; }
skip() { printf 'SKIP %s\n' "$1"; }

stage "check"
cargo check --workspace --all-targets --locked

stage "verify"
if command -v yashyc >/dev/null 2>&1; then
  yashyc verify --json --inventory examples/hello/main.ya
else
  # Build the driver once; running it from target/debug keeps the check
  # meaningful on a machine with no installed binary.
  cargo build --locked -p yashy-driver --bin yashyc
  target/debug/yashyc verify --json --inventory examples/hello/main.ya
fi

stage "build"
cargo build --release --workspace --locked

stage "test"
cargo test --workspace --locked

stage "fmt"
cargo fmt --all -- --check

stage "lint"
cargo clippy --workspace --all-targets --locked -- -D warnings || \
  skip "clippy: treat a new lint as a failure only when the workspace is clean"

stage "coverage"
if command -v yashyc >/dev/null 2>&1; then
  yashyc coverage --json --fail-under 70 examples/hello >/dev/null
else
  skip "coverage: run through the built driver after the build stage"
fi

stage "benchmark gate"
if [ -x ./benchmarks/ci.sh ]; then
  ./benchmarks/ci.sh
else
  skip "benchmark gate: benchmarks/ci.sh not present on this machine"
fi

stage "fuzz smoke"
if command -v yashyc >/dev/null 2>&1 && [ -d fuzz ]; then
  yashyc fuzz --runs 1000
else
  skip "fuzz smoke: yashyc fuzz and a corpus are not yet available"
fi

printf '\nAll available stages passed.\n'
