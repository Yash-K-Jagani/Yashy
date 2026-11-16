# Phase 12 — Reach and Interop

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Put a Yashy binary where the work already is: a browser tab, a Node
process, a Python host, a C or C++ codebase, an ARM server, a static musl image
— and let each of those call into Yashy and back without a hand-written glue
layer.

**Outcome:** WASM32 (`unknown` and WASI) with a working `hello`, a browser
playground that compiles in the tab, JS interop with a generated binding, a
`yashyc bindgen` that turns a C header into checkable Yashy declarations and
back, C++ through the C ABI, Python embedding and Node N-API packages, a
cross-target matrix with `aarch64` Linux/Windows, macOS `x86_64`/`aarch64` and
musl static, portable SIMD vectors with a benchmark, `dlopen` on both OSes, and
an FFI conformance suite.

**Time:** ~8 weeks.
**Depends on:** Phase 11B gate.
**Spec:** [`../specs/2026-11-15-yashy-2.0-design.md`](../specs/2026-11-15-yashy-2.0-design.md) §6.7, §14 (calls 7–9).

## Why this phase

A language's reach is measured at the boundary, and Yashy's boundary is already
its strongest claim: ownership is explicit, `repr(C)` is a keyword form, and
`extern "C"` with `link_name` and variadics exists at 1.0. What is missing is
everything around the boundary: no second architecture, no WASM, no way to
generate or verify a binding, no documented ABI conformance corpus, and no SIMD.

The phase is deliberately after performance. SIMD is a codegen feature, and
shipping it before the inliner and bounds work would add a vector path to a
pipeline still being reshaped. WASM is deliberately last and cuttable — but its
browser proof is the demo a stranger can run in ten seconds, which is worth
having before the AI phase writes documentation about the language.

## Starting point

| Fact | 1.0 value |
|---|---|
| Triples | six, `x86_64` focused; no `aarch64`, no musl static |
| WASM | none |
| Playground | `yashyc playground` runs a local page; no in-browser WASM compile |
| JS interop | none |
| `bindgen` | none; C declarations are hand-written |
| C++ | C ABI only by hand |
| Python/Node | none |
| SIMD | none; scalar code only |
| `repr` | `repr(C)` only (Phase 4 adds the rest) |
| `dlopen` | none |
| macOS | unsigned, unclaimed |
| FFI tests | a handful of `extern` programs |

## Deliverables

### Task 1: The target matrix

**Files:**
- Modify: `crates/yashy-driver/src/target.rs`, `crates/yashy-driver/src/toolchain.rs`, `crates/yashy-driver/src/cfg.rs`
- Create: `scripts/targets/*.sh`, `docs/targets.md`
- Test: `crates/yashy-driver/tests/target.rs`, `crates/yashy-driver/tests/cross.rs`

**Interfaces:**
- `Target { triple: String, arch: Arch, os: Os, env: Env, data_layout: DataLayout, cfg_keys: BTreeSet<String>, libs: Vec<String> }`
- `yashyc build --target <triple>`; `yashyc doctor --targets`

- [ ] **Step 1: Write the failing test.** Build and run `hello` for
  `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`,
  `x86_64-pc-windows-msvc`, `aarch64-pc-windows-msvc`,
  `x86_64-apple-darwin`, `aarch64-apple-darwin`,
  `x86_64-unknown-linux-musl`, `wasm32-unknown-unknown` and
  `wasm32-wasi`. Assert each artifact has the expected format, and that a target
  whose linker or SDK is missing fails with one diagnostic naming what to
  install, never a raw linker error.
- [ ] **Step 2: One triple is the whole build's fact.** `cfg` keys, the data
  layout, the platform libraries and `clang --target` all read the same `Target`.
  A `cfg` key the target does not define is `E2202`; a target whose layout changed
  is a test failure, not a silent ABI break.
- [ ] **Step 3: Native tests, cross tests, and honest skips.** A target with an
  emulator runs its programs under it (`qemu-aarch64`, `wasmtime`); a target
  without one builds and inspects only, and the gate says so. No target is
  marked supported on a build alone.
- [ ] **Step 4: Commit.**

### Task 2: WASM32 and WASI

**Files:**
- Modify: `crates/yashy-codegen/src/ir.rs`, `crates/yashy-codegen/src/memory.rs` (pointer width, alignment)
- Create: `crates/yashy-runtime/wasm/` (the shim's WASM build), `crates/yashy-stdlib/yashy/os/wasi.ya`
- Modify: `crates/yashy-driver/src/main.rs` (build/link), `crates/yashy-driver/src/toolchain.rs` (wasm-ld discovery)
- Test: `crates/yashy-driver/tests/wasm.rs`, `crates/yashy-driver/tests/programs/wasm/`

- [ ] **Step 1: Write the failing test.** Compile `hello` to
  `wasm32-wasi` and run it under `wasmtime`; assert stdout. Compile a program
  that does file I/O and one that reads `args`; assert WASI preopens and
  arguments work. Compile to `wasm32-unknown-unknown` and assert the module
  exports `main` and allocates through the imported memory.
- [ ] **Step 2: Pointer width is a target fact, not a constant.** Every
  `int`/pointer width decision reads `Target`. A 32-bit build that emits a
  64-bit pointer is caught by a layout test on the WASM target, not by a user.
- [ ] **Step 3: No hidden host.** The WASM build has no ambient filesystem,
  clock or environment: each is a WASI import or a Phase 4 capability that the
  program declares. A program that reads one without it is a diagnostic.
- [ ] **Step 4: The size budget.** `hello.wasm` is under 100 KiB and allocates
  no more than the program needs; the number is recorded and gated at 10%.
- [ ] **Step 5: Commit.**

### Task 3: JS interop and the browser playground

**Files:**
- Create: `crates/yashy-driver/src/wasm_bind.rs` (JS glue generator)
- Create: `packages/wasm/src/{bind,js}.ya`, `playground/` (the browser page)
- Modify: `crates/yashy-driver/src/main.rs` (`playground --wasm`)
- Test: `crates/yashy-driver/tests/wasm_bind.rs`, `playground/tests/` (headless)

- [ ] **Step 1: Write the failing test.** An `extern "js"` function returning
  a string and one taking a slice; the generated `.js` glue loads the module,
  calls both, and returns the expected values under Node. In the browser page,
  type a program, press Run, and see stdout — asserted by a headless browser
  test.
- [ ] **Step 2: The ABI is generated, not remembered.** Strings cross as
  pointer + length with an explicit ownership transfer; a slice is borrowed for
  the call and freed by the callee; a JS exception becomes a `Result`, never a
  trap. The generated glue is a build artifact with a header naming the
  compiler version.
- [ ] **Step 3: The playground is a real compiler.** It runs the same front end
  compiled to WASM, in a worker, with a 2-second timeout and no network. A
  program that needs the filesystem says so and is refused. The page works
  offline from a static directory.
- [ ] **Step 4: Commit.**

### Task 4: `yashyc bindgen`

**Files:**
- Create: `crates/yashy-driver/src/bindgen.rs`, `crates/yashy-driver/src/c_header.rs`
- Create: `docs/bindgen.md`
- Test: `crates/yashy-driver/tests/bindgen.rs`, `crates/yashy-driver/tests/programs/ffi/bindgen/`

**Interfaces:**
- `bindgen(header: &Path, opts: BindgenOptions) -> Result<Bindings, BindgenError>`
- `BindgenOptions { include: Vec<PathBuf>, defines: Vec<String>, allowlist: Vec<String>, blocklist: Vec<String>, target: Triple }`

- [ ] **Step 1: Write the failing test.** Run `bindgen` on a header with a
  struct, an enum, a typedef, a function pointer, a variadic function, an opaque
  type, a bitfield and a macro constant; compile the generated Yashy; call the
  C library and assert the round trip. Then run `bindgen --verify` and assert it
  checks every generated declaration against the header's actual layout.
- [ ] **Step 2: Layout is checked, not assumed.** Every generated struct carries
  `repr(C)` and a static assertion that `size_of`/`offset_of` match the header on
  the target. A mismatch is a bindgen error with the field named. This is the
  Phase 4 layout table doing real work.
- [ ] **Step 3: Generated code is reviewable.** One file, sorted, stable across
  runs, with a header naming the header path, the triple and the bindgen version;
  `--check` in CI fails when the checked-in bindings drift. Bitfields become
  accessor functions with tests, not a silent packed read.
- [ ] **Step 4: Unsafe is explicit.** Every pointer parameter, every
  dereference and every macro that allocates lands in an `unsafe fn` with a
  comment naming the C contract; `verify --inventory` counts them.
- [ ] **Step 5: Commit.**

### Task 5: C++, Python and Node

**Files:**
- Create: `examples/interop/cpp/`, `packages/python/`, `packages/node/`
- Create: `docs/interop.md`
- Test: `examples/interop/cpp/tests/`, `packages/python/tests/`, `packages/node/tests/`

- [ ] **Step 1: Write the failing test.** A C++ class exposed through a
  `extern "C"` shim is called from Yashy and the reverse; a Yashy function is
  imported from Python and called 10,000 times in a loop with a string and a
  struct; the same through Node N-API. Assert values, no leak, and that the host
  runtime's GC/allocator and Yashy's allocator do not free each other's memory.
- [ ] **Step 2: The C ABI is the contract.** C++ interop is a C shim with
  `repr(C)` types, not a C++ ABI promise. Python and Node bindings are generated
  from the same `bindgen`/`wasm_bind` machinery, so there is one ABI description
  and three consumers.
- [ ] **Step 3: Ownership crosses explicitly.** A string or array handed to a
  host is either copied or transferred, and the choice is a function name
  (`into_python` vs `borrow_for_python`), never a default. A host exception
  becomes a `Result`.
- [ ] **Step 4: Commit.**

### Task 6: Portable SIMD

**Files:**
- Create: `crates/yashy-stdlib/yashy/simd.ya`
- Modify: `crates/yashy-codegen/src/expr_gen.rs`, `crates/yashy-codegen/src/pass.rs`
- Test: `crates/yashy-driver/tests/programs/simd/`
- Bench: `benchmarks/simd.bench`

**Interfaces:**
- `Simd[T, N]` for `T ∈ {i8..i64, u8..u64, f32, f64}` and `N ∈ {2,4,8,16,32,64}`
  as a fixed-width value type (the name does not collide with `Vec[T]`)
- `load`, `store`, `splat`, `add`, `sub`, `mul`, `min`, `max`, `shuffle`,
  `reduce_add`, with a mask type and `select`

- [ ] **Step 1: Write the failing test.** A dot product, a byte-sum and a
  saturating clamp, each asserted against the scalar result; a target without a
  vector unit still compiles and produces the same answer.
- [ ] **Step 2: Portable first, fast second.** The surface is a fixed-width
  value type; lowering is LLVM vector IR. There is no `unsafe` intrinsic and no
  target-specific builtin in the public surface. A width the target lacks is
  lowered to two halves, not refused.
- [ ] **Step 3: The benchmark is the gate.** A `simd.bench` row shows the speedup
  over the scalar loop on both OSes; a row that is not faster is investigated or
  the surface is reduced. Bounds and overflow checks still apply to scalar
  fallbacks.
- [ ] **Step 4: Commit.**

### Task 7: FFI conformance and `dlopen`

**Files:**
- Create: `crates/yashy-driver/tests/ffi_conformance.rs`, `crates/yashy-driver/tests/programs/ffi/conformance/`
- Modify: `crates/yashy-runtime/` (the `dlopen` shim per OS), `crates/yashy-stdlib/yashy/os.ya`
- Create: `docs/ffi.md`

- [ ] **Step 1: Write the failing test.** A conformance matrix over: integer
  widths and signedness, `bool` representation, enum repr, struct by value and
  by pointer, arrays, function pointers, variadics, null pointers as `Option`,
  ABI strings (`"C"`, `"system"`, `"stdcall"`, `"fastcall"`, `"vectorcall"`), and
  callbacks from C into Yashy. Each case is a C file and a Yashy file with the
  same expected output, run on both OSes and both architectures. An ABI string
  the target does not support is a diagnostic naming both, not a link error.
- [ ] **Step 2: `dlopen` is a capability.** Load a shared library by path at
  run time, resolve a symbol by name, and refuse a missing symbol with a `Result`
  naming it. Under the Phase 13 sandbox, `dlopen` requires the `ffi` capability
  and a path inside the allowlist.
- [ ] **Step 3: The matrix is generated.** `docs/ffi.md` renders the table from
  the test cases, so a new ABI case appears in the docs when it appears in the
  corpus.
- [ ] **Step 4: Commit.**

## Gates

| Gate | Command | Passes when |
|---|---|---|
| Targets | `cargo test -p yashy-driver --test target --test cross` | every claimed triple builds; emulated targets run; an absent SDK is a diagnostic |
| WASM | `cargo test -p yashy-driver --test wasm` | `hello` runs in `wasmtime` and Node; `wasm32-unknown-unknown` exports main; size < 100 KiB |
| Playground | `playground/tests` (headless) | type, run, see stdout, offline, 2 s timeout |
| Bindgen | `cargo test -p yashy-driver --test bindgen` | header round-trips; `--verify` catches a layout mismatch; `--check` catches drift |
| Interop | `examples/interop/cpp/tests`, `packages/{python,node}/tests` | C++, Python and Node calls pass; no cross-allocator free |
| SIMD | `yashyc bench simd --check-baseline` | correct on both targets; speedup recorded |
| FFI conformance | `cargo test -p yashy-driver --test ffi_conformance` | every ABI case passes on both OSes and both architectures |
| No regression | `cargo test --workspace` | green on Windows and Linux |
| No P0 | `docs/triage.md` | no P0 |

## Demo

```console
$ yashyc build examples/hello/main.ya --target wasm32-wasi
built hello.wasm  38,912 B
$ wasmtime hello.wasm
hello, world

$ yashyc bindgen /usr/include/sqlite3.h --allowlist 'sqlite3_*' -o sqlite3.ya
wrote sqlite3.ya  142 declarations  size/offset checked  unsafe: 96
$ yashyc build sqlite3.ya --check
layout ok  142/142

$ yashyc run examples/simd/main.ya --bench
scalar   412.8 ms
simd      52.1 ms   7.9×
```

```console
$ yashyc playground --wasm
serving http://127.0.0.1:8080  (offline, compiler in the tab)
```

## Out of scope

- Android, iOS and bare-metal targets.
- A C++ ABI (name mangling, exceptions, templates); C++ is exposed through C.
- A Python or Node package published to a public index before the Phase 14
  release.
- Auto-vectorisation of arbitrary loops; that is Phase 11B's proved patterns.
- A hosted playground with accounts or storage.
- Claiming macOS before the Phase 14 signed archive.

## Risks

| Risk | Mitigation |
|---|---|
| A cross target builds but cannot run | Emulated targets run; a build-only target is labelled and not claimed as supported |
| WASM grows into a second runtime | The same front end and codegen, a WASI shim, no ambient host; size budget |
| The playground becomes a fork | It compiles the same compiler; no separate parser or stdlib |
| Bindgen emits a wrong layout | `repr(C)` + static assertions + `--verify`; a mismatch names the field |
| Host and Yashy allocators free each other's memory | Copy or transfer is explicit in the function name; a test with both allocators |
| SIMD changes results | Scalar differential on both targets; portable surface only |
| `dlopen` widens the attack surface | Capability-gated under the sandbox; path allowlist; a `Result` on a missing symbol |
| macOS slips again | macOS stays unclaimed until the signed archive exists; the matrix says "build only" until then |
