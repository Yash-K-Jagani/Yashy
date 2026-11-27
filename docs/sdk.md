# Yashy SDK

`yashy-sdk` is a Rust crate that exposes the compiler's verbs as library calls.
It is a thin adapter over `yashy-driver`: no parser, checker or code generator
lives in the SDK, so a library client and `yashyc` read the same facts from the
same pipeline.

```rust
use std::path::Path;
use yashy_sdk::{check, explain, index, review};

let result = check(Path::new("main.ya"), source, false);
assert!(result.success);
let json = index(Path::new(".")).expect("a workspace index").render();
let entry = explain("E0100").expect("a catalog entry").render();
```

The verbs:

| Call | Same CLI verb |
|---|---|
| `check(path, source, enforce)` | `yashyc check [--json]` |
| `verify(path, source)` | `yashyc verify` — `check` with contracts enforced |
| `build(source, entry, out, options)` | `yashyc build` |
| `index(root)` | `yashyc index --json` |
| `review(root, baseline)` | `yashyc review --json` |
| `impact(root, baseline)` | `yashyc impact --json` |
| `context(root, task, budget)` | `yashyc context --json` |
| `explain(code)` | `yashyc explain --json` |

Every JSON value uses the CLI's schema-v1 envelope. `CheckResult` carries the
typed diagnostics and renders them to the same JSON the CLI prints. `build`
takes the driver's `BuildOptions` verbatim, so flags such as `--release` and
`--target` mean the same thing in both surfaces.

`workspace_root(path)` walks up to the nearest `yashy.toml`, which is how a
client finds the workspace a source file belongs to.

The crate is `#![forbid(unsafe_code)]`. The CLI, LSP and MCP server still call
`yashy-driver` directly today; moving those call sites to the SDK is the
remaining half of the "one embedding surface" obligation in the Phase 13 plan.
