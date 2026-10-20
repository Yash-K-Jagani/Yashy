#!/bin/sh
# Run every example in this tutorial through the compiler.
#
# The checks live in Rust, not here: `crates/otter-driver/tests/tutorial.rs`
# extracts each ```otter block, compiles it with the `otter` binary, runs it,
# and compares stdout with the block's `// =>` lines. This script is the entry
# point a reader can remember.
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$root"

exec cargo test -p otter-driver --test tutorial
