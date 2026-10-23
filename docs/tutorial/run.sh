#!/bin/sh
# Run every example in this tutorial through the compiler.
#
# The checks live in Rust, not here: `crates/yashy-driver/tests/tutorial.rs`
# extracts each ```yashy block, compiles it with the `yashy` binary, runs it,
# and compares stdout with the block's `// =>` lines. This script is the entry
# point a reader can remember.
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$root"

exec cargo test -p yashy-driver --test tutorial
