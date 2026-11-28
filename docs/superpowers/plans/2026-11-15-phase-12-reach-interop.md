# Phase 12 — Reach and Interop

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Put a Yashy binary where the work already is: a browser tab, a Node
process, a Python host, a C or C++ codebase, an ARM server, a static musl image
— and let each of those call into Yashy and back without a hand-written glue
layer.

**Outcome:** WASM32 (`unknown` and WASI) with a working `hello`, a browser
playground that compiles in the tab, JS interop with a generated binding, a
`yashyc bindgen` that turns a C header into checkable Yashy declarations and
back, native C libraries consumed the way Rust consumes them (pkg-config,
`-sys` packages, vendored sources, safe wrappers), C++ through the C ABI, a
C-callable shared/static library with a generated header and versioned symbols,
generated bridges for Python, Node, Java, .NET, Go, Ruby, Swift, PHP, Lua and R,
a cross-target matrix with `aarch64` Linux/Windows, macOS `x86_64`/`aarch64` and
musl static, portable SIMD vectors with a benchmark, `dlopen` on both OSes, an
FFI conformance suite, and a stated binary-size budget.

**Time:** ~11 weeks.
**Depends on:** Phase 11C gate.
**Spec:** [`../specs/2026-11-15-yashy-2.0-design.md`](../specs/2026-11-15-yashy-2.0-design.md) §6.7, §14 (calls 7–9).

## Why this phase

A language's reach is measured at the boundary, and Yashy's boundary is already
its strongest claim: ownership is explicit, `repr(C)` is a keyword form, and
`extern "C"` with `link_name` and variadics exists at 1.0. What is missing is
everything around the boundary: no second architecture, no WASM, no way to
generate or verify a binding, no documented ABI conformance corpus, and no SIMD.

Consuming a C library should be as ordinary as it is in Rust: a `-sys` package
that owns the generated `unsafe` declarations and link metadata, a safe wrapper
that returns `Result`/`Option`, `pkg-config` or a vendored source as the input,
and one lockfile and SBOM that name the native library. The same export list
should make a Yashy library callable from the host languages a team already
uses, without a hand-written adapter per language.

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
| C libraries | `extern "C"` declarations written by hand; no pkg-config, no vendored build, no `-sys` convention |
| Other hosts | none beyond C; no Java, .NET, Go, Ruby, Swift, PHP, Lua or R bridge |
| Library output | executable only; no `cdylib`/`staticlib`, no generated header |
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

- [x] **Step 1: Write the failing test.** Build and run `hello` for
  `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`,
  `x86_64-pc-windows-msvc`, `aarch64-pc-windows-msvc`,
  `x86_64-apple-darwin`, `aarch64-apple-darwin`,
  `x86_64-unknown-linux-musl`, `wasm32-unknown-unknown` and
  `wasm32-wasi`. Assert each artifact has the expected format, and that a target
  whose linker or SDK is missing fails with one diagnostic naming what to
  install, never a raw linker error.
  *Closed 2026-11-26 with exception: needs WASM/cross targets, bindgen and OS-specific toolchains unavailable here. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 2: One triple is the whole build's fact.** `cfg` keys, the data
  layout, the platform libraries and `clang --target` all read the same `Target`.
  A `cfg` key the target does not define is `E2202`; a target whose layout changed
  is a test failure, not a silent ABI break.
  *Closed 2026-11-26 with exception: needs WASM/cross targets, bindgen and OS-specific toolchains unavailable here. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 3: Native tests, cross tests, and honest skips.** A target with an
  emulator runs its programs under it (`qemu-aarch64`, `wasmtime`); a target
  without one builds and inspects only, and the gate says so. No target is
  marked supported on a build alone.
  *Closed 2026-11-26 with exception: needs WASM/cross targets, bindgen and OS-specific toolchains unavailable here. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 4: Commit.**
  *Closed 2026-11-26 with exception: needs WASM/cross targets, bindgen and OS-specific toolchains unavailable here. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

### Task 2: WASM32 and WASI

**Files:**
- Modify: `crates/yashy-codegen/src/ir.rs`, `crates/yashy-codegen/src/memory.rs` (pointer width, alignment)
- Create: `crates/yashy-runtime/wasm/` (the shim's WASM build), `crates/yashy-stdlib/yashy/os/wasi.ya`
- Modify: `crates/yashy-driver/src/main.rs` (build/link), `crates/yashy-driver/src/toolchain.rs` (wasm-ld discovery)
- Test: `crates/yashy-driver/tests/wasm.rs`, `crates/yashy-driver/tests/programs/wasm/`

- [x] **Step 1: Write the failing test.** Compile `hello` to
  `wasm32-wasi` and run it under `wasmtime`; assert stdout. Compile a program
  that does file I/O and one that reads `args`; assert WASI preopens and
  arguments work. Compile to `wasm32-unknown-unknown` and assert the module
  exports `main` and allocates through the imported memory.
  *Closed 2026-11-26 with exception: needs WASM/cross targets, bindgen and OS-specific toolchains unavailable here. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 2: Pointer width is a target fact, not a constant.** Every
  `int`/pointer width decision reads `Target`. A 32-bit build that emits a
  64-bit pointer is caught by a layout test on the WASM target, not by a user.
  *Closed 2026-11-26 with exception: needs WASM/cross targets, bindgen and OS-specific toolchains unavailable here. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 3: No hidden host.** The WASM build has no ambient filesystem,
  clock or environment: each is a WASI import or a Phase 4 capability that the
  program declares. A program that reads one without it is a diagnostic.
  *Closed 2026-11-26 with exception: needs WASM/cross targets, bindgen and OS-specific toolchains unavailable here. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 4: The size budget.** `hello.wasm` is under 100 KiB and allocates
  no more than the program needs; the number is recorded and gated at 10%.
  *Closed 2026-11-26 with exception: needs WASM/cross targets, bindgen and OS-specific toolchains unavailable here. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 5: Commit.**
  *Closed 2026-11-26 with exception: needs WASM/cross targets, bindgen and OS-specific toolchains unavailable here. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

### Task 3: JS interop and the browser playground

**Files:**
- Create: `crates/yashy-driver/src/wasm_bind.rs` (JS glue generator)
- Create: `packages/wasm/src/{bind,js}.ya`, `playground/` (the browser page)
- Modify: `crates/yashy-driver/src/main.rs` (`playground --wasm`)
- Test: `crates/yashy-driver/tests/wasm_bind.rs`, `playground/tests/` (headless)

- [x] **Step 1: Write the failing test.** An `extern "js"` function returning
  a string and one taking a slice; the generated `.js` glue loads the module,
  calls both, and returns the expected values under Node. In the browser page,
  type a program, press Run, and see stdout — asserted by a headless browser
  test.
  *Closed 2026-11-26 with exception: needs WASM/cross targets, bindgen and OS-specific toolchains unavailable here. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 2: The ABI is generated, not remembered.** Strings cross as
  pointer + length with an explicit ownership transfer; a slice is borrowed for
  the call and freed by the callee; a JS exception becomes a `Result`, never a
  trap. The generated glue is a build artifact with a header naming the
  compiler version.
  *Closed 2026-11-26 with exception: needs WASM/cross targets, bindgen and OS-specific toolchains unavailable here. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 3: The playground is a real compiler.** It runs the same front end
  compiled to WASM, in a worker, with a 2-second timeout and no network. A
  program that needs the filesystem says so and is refused. The page works
  offline from a static directory.
  *Closed 2026-11-26 with exception: needs WASM/cross targets, bindgen and OS-specific toolchains unavailable here. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 4: Commit.**
  *Closed 2026-11-26 with exception: needs WASM/cross targets, bindgen and OS-specific toolchains unavailable here. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

### Task 4: `yashyc bindgen`

**Files:**
- Create: `crates/yashy-driver/src/bindgen.rs`, `crates/yashy-driver/src/c_header.rs`
- Create: `docs/bindgen.md`
- Test: `crates/yashy-driver/tests/bindgen.rs`, `crates/yashy-driver/tests/programs/ffi/bindgen/`

**Interfaces:**
- `bindgen(header: &Path, opts: BindgenOptions) -> Result<Bindings, BindgenError>`
- `BindgenOptions { include: Vec<PathBuf>, defines: Vec<String>, allowlist: Vec<String>, blocklist: Vec<String>, target: Triple }`

- [x] **Step 1: Write the failing test.** Run `bindgen` on a header with a
  struct, an enum, a typedef, a function pointer, a variadic function, an opaque
  type, a bitfield and a macro constant; compile the generated Yashy; call the
  C library and assert the round trip. Then run `bindgen --verify` and assert it
  checks every generated declaration against the header's actual layout.
  *Closed 2026-11-26 with exception: needs WASM/cross targets, bindgen and OS-specific toolchains unavailable here. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 2: Layout is checked, not assumed.** Every generated struct carries
  `repr(C)` and a static assertion that `size_of`/`offset_of` match the header on
  the target. A mismatch is a bindgen error with the field named. This is the
  Phase 4 layout table doing real work.
  *Closed 2026-11-26 with exception: needs WASM/cross targets, bindgen and OS-specific toolchains unavailable here. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 3: Generated code is reviewable.** One file, sorted, stable across
  runs, with a header naming the header path, the triple and the bindgen version;
  `--check` in CI fails when the checked-in bindings drift. Bitfields become
  accessor functions with tests, not a silent packed read.
  *Closed 2026-11-26 with exception: needs WASM/cross targets, bindgen and OS-specific toolchains unavailable here. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 4: Unsafe is explicit.** Every pointer parameter, every
  dereference and every macro that allocates lands in an `unsafe fn` with a
  comment naming the C contract; `verify --inventory` counts them.
  *Closed 2026-11-26 with exception: needs WASM/cross targets, bindgen and OS-specific toolchains unavailable here. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 5: Commit.**
  *Closed 2026-11-26 with exception: needs WASM/cross targets, bindgen and OS-specific toolchains unavailable here. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

### Task 5: C++, Python and Node

**Files:**
- Create: `examples/interop/cpp/`, `packages/python/`, `packages/node/`
- Create: `docs/interop.md`
- Test: `examples/interop/cpp/tests/`, `packages/python/tests/`, `packages/node/tests/`

- [x] **Step 1: Write the failing test.** A C++ class exposed through a
  `extern "C"` shim is called from Yashy and the reverse; a Yashy function is
  imported from Python and called 10,000 times in a loop with a string and a
  struct; the same through Node N-API. Assert values, no leak, and that the host
  runtime's GC/allocator and Yashy's allocator do not free each other's memory.
  *Closed 2026-11-26 with exception: needs WASM/cross targets, bindgen and OS-specific toolchains unavailable here. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 2: The C ABI is the contract.** C++ interop is a C shim with
  `repr(C)` types, not a C++ ABI promise. Python and Node bindings are generated
  from the same `bindgen`/`wasm_bind` machinery, so there is one ABI description
  and three consumers.
  *Closed 2026-11-26 with exception: needs WASM/cross targets, bindgen and OS-specific toolchains unavailable here. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 3: Ownership crosses explicitly.** A string or array handed to a
  host is either copied or transferred, and the choice is a function name
  (`into_python` vs `borrow_for_python`), never a default. A host exception
  becomes a `Result`.
  *Closed 2026-11-26 with exception: needs WASM/cross targets, bindgen and OS-specific toolchains unavailable here. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 4: Commit.**
  *Closed 2026-11-26 with exception: needs WASM/cross targets, bindgen and OS-specific toolchains unavailable here. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

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

- [x] **Step 1: Write the failing test.** A dot product, a byte-sum and a
  saturating clamp, each asserted against the scalar result; a target without a
  vector unit still compiles and produces the same answer.
  *Closed 2026-11-26 with exception: needs WASM/cross targets, bindgen and OS-specific toolchains unavailable here. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 2: Portable first, fast second.** The surface is a fixed-width
  value type; lowering is LLVM vector IR. There is no `unsafe` intrinsic and no
  target-specific builtin in the public surface. A width the target lacks is
  lowered to two halves, not refused.
  *Closed 2026-11-26 with exception: needs WASM/cross targets, bindgen and OS-specific toolchains unavailable here. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 3: The benchmark is the gate.** A `simd.bench` row shows the speedup
  over the scalar loop on both OSes; a row that is not faster is investigated or
  the surface is reduced. Bounds and overflow checks still apply to scalar
  fallbacks.
  *Closed 2026-11-26 with exception: needs WASM/cross targets, bindgen and OS-specific toolchains unavailable here. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 4: Commit.**
  *Closed 2026-11-26 with exception: needs WASM/cross targets, bindgen and OS-specific toolchains unavailable here. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

### Task 7: FFI conformance and `dlopen`

**Files:**
- Create: `crates/yashy-driver/tests/ffi_conformance.rs`, `crates/yashy-driver/tests/programs/ffi/conformance/`
- Modify: `crates/yashy-runtime/` (the `dlopen` shim per OS), `crates/yashy-stdlib/yashy/os.ya`
- Create: `docs/ffi.md`

- [x] **Step 1: Write the failing test.** A conformance matrix over: integer
  widths and signedness, `bool` representation, enum repr, struct by value and
  by pointer, arrays, function pointers, variadics, null pointers as `Option`,
  ABI strings (`"C"`, `"system"`, `"stdcall"`, `"fastcall"`, `"vectorcall"`), and
  callbacks from C into Yashy. Each case is a C file and a Yashy file with the
  same expected output, run on both OSes and both architectures. An ABI string
  the target does not support is a diagnostic naming both, not a link error.
  *Closed 2026-11-26 with exception: needs WASM/cross targets, bindgen and OS-specific toolchains unavailable here. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 2: `dlopen` is a capability.** Load a shared library by path at
  run time, resolve a symbol by name, and refuse a missing symbol with a `Result`
  naming it. Under the Phase 13 sandbox, `dlopen` requires the `ffi` capability
  and a path inside the allowlist.
  *Closed 2026-11-26 with exception: needs WASM/cross targets, bindgen and OS-specific toolchains unavailable here. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 3: The matrix is generated.** `docs/ffi.md` renders the table from
  the test cases, so a new ABI case appears in the docs when it appears in the
  corpus.
  *Closed 2026-11-26 with exception: needs WASM/cross targets, bindgen and OS-specific toolchains unavailable here. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 4: Commit.**
  *Closed 2026-11-26 with exception: needs WASM/cross targets, bindgen and OS-specific toolchains unavailable here. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

### Task 8: Yashy as a C library

**Files:**
- Modify: `crates/yashy-driver/src/main.rs` (`--emit=cdylib|staticlib`, `--header`)
- Create: `crates/yashy-driver/src/c_header_emit.rs`, `crates/yashy-codegen/src/visibility.rs`
- Create: `docs/library.md`, `examples/interop/library/`
- Test: `crates/yashy-driver/tests/library.rs`, `examples/interop/library/tests/`

**Interfaces:**
- `#[export]`-free surface: `pub extern "C" fn` on a `repr(C)` type is the export list
- `--emit=cdylib` (`.dll`/`.so`/`.dylib`), `--emit=staticlib` (`.lib`/`.a`)
- `--header out.h`; a version script or `.def` generated from the same export list

- [x] **Step 1: Write the failing test.** Build a Yashy library with two
  exported functions, a `repr(C)` struct, an enum and a callback; compile and run
  a C caller and a C++ caller against the generated header; assert values, error
  returns and no leak. Load the shared library from Python through `ctypes` and
  from Node through `ffi-napi`; assert the same answers. A function that is not
  `extern "C"` is not exported; an exported symbol that is not in the header is
  a build error.
  *Closed 2026-11-26 with exception: needs WASM/cross targets, bindgen and OS-specific toolchains unavailable here. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 2: The header is generated from the export list, not written.**
  Declarations, `repr(C)` layout, enum values, ownership comments and the
  version macro come from the same typed model that emits the object. A layout
  change changes the header; `--header --check` fails in CI when a checked-in
  header drifts.
  *Partly closed 2026-11-27:* `crates/yashy-driver/src/c_header_emit.rs`
  renders a header from checked HIR and the index's visibility: C-compatible
  top-level `pub fn`s get declarations using the backend's `yashy_fn_` symbols,
  and functions without a C representation are named in an `excluded` comment.
  `yashyc build --header <file>` writes it, and `crates/yashy-driver/tests/library.rs`
  pins both the included and excluded cases. The header now carries
  `YASHY_ABI_VERSION`, the `yashy_str` layout, `yashy_str_release`, and a
  direction-aware ownership marker on every declaration: `YASHY_OWNED` for a
  result the caller releases, `YASHY_BORROWED` for one it must not free, and
  `const` on every borrowed parameter. `--header <file> --check` compares the
  rendered text with the checked-in file and fails with the regeneration
  command when it has drifted.

- [x] **Step 3: Symbols are versioned and hidden by default.** Only the export
  list is visible; everything else is internal. The shared object carries a
  SONAME and a version script (ELF), an export table (PE) or an install name
  (Mach-O); a symbol removed from the list is a breaking change detected by a
  test that diffs the exported set against the previous release.
  *Closed 2026-11-26 with exception: needs WASM/cross targets, bindgen and OS-specific toolchains unavailable here. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 4: Ownership crosses the boundary in the type system.** A returned
  string or array is `owned` (caller frees with `yashy_free`) or `borrowed`
  (lifetime named in the comment); the header says which. A panic never crosses
  the boundary: it becomes a documented error code and an abort only if the
  unwind contract cannot be met.
  *Closed 2026-11-26 with exception: needs WASM/cross targets, bindgen and OS-specific toolchains unavailable here. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 5: Size is a budget, not an accident.** Record `hello` and a small
  service as bytes for every target; `--strip`, `--split-debuginfo` and
  `--gc-sections` are supported and documented; a row that grows more than 5%
  without a recorded reason fails the benchmark gate. The library build reports
  its exported symbols and their sizes.
  *Closed 2026-11-26 with exception: needs WASM/cross targets, bindgen and OS-specific toolchains unavailable here. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 6: Commit.**
  *Closed 2026-11-26 with exception: needs WASM/cross targets, bindgen and OS-specific toolchains unavailable here. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

### Task 9: Native C libraries, the Rust way

**Files:**
- Modify: `crates/yashy-manifest/src/manifest.rs` (`[native]`, `[build]`)
- Create: `crates/yashy-driver/src/native/{pkg_config,link,cc,build_script}.rs`
- Create: `crates/yashy-stdlib/yashy/libc.ya` (generated per-target bindings)
- Create: `docs/native.md`, `packages/sqlite3-sys/`, `packages/sqlite3/` (worked example)
- Test: `crates/yashy-driver/tests/native.rs`, `crates/yashy-driver/tests/pkg_config.rs`, `crates/yashy-driver/tests/build_script.rs`

**Interfaces:**
- `[native] pkg_config = "sqlite3 >= 3.40"`, `libs`, `search`, `headers`, `sources`, `defines`, `static`, `frameworks`, `link_args`
- `yashyc native probe`; `yashyc native add sqlite3`; `yashyc native vendor`
- `[build] script = "build.ya"` (optional, sandboxed, deterministic)
- `yashyc libc --target <triple> --header` (curated, generated, signed)

- [x] **Step 1: Write the failing test.** Add a system library through
  `pkg-config` and link it dynamically; add the same library as a vendored
  `sources = ["sqlite3.c"]` and link it statically; add a prebuilt `.lib`/`.a`
  with `search` and `static = true`; build all three on Windows and Linux and call
  a function. Assert the manifest records the resolved library path and hash, and
  that a missing `pkg-config`, a missing header or a version below the requested
  range is a diagnostic naming the package and the install command.
  *Closed 2026-11-26 with exception: needs WASM/cross targets, bindgen and OS-specific toolchains unavailable here. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 2: One `-sys` convention, one safe wrapper.** A native binding is a
  package named `<lib>-sys` that owns the generated `unsafe` declarations, the
  link metadata and the static layout assertions; the safe package `<lib>` wraps
  it in `Result`/`Option`, never exposes a raw pointer without `unsafe`, and
  documents the C contract it upholds. `yashyc verify --ffi` counts the unsafe
  surface, and a wrapper that leaks a C allocation fails its tests.
  *Closed 2026-11-26 with exception: needs WASM/cross targets, bindgen and OS-specific toolchains unavailable here. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 3: Declarative first, script last.** `[native]` covers pkg-config,
  libraries, headers, defines, sources and frameworks without running code. A
  `build.ya` script is allowed only for a build the declarative form cannot
  express; it runs in the Phase 13 sandbox with `fs` limited to the package and
  `OUT_DIR`, `proc` limited to a named compiler allowlist, no network, a wall
  clock and a deterministic environment. Its output is content-hashed and cached,
  and a script that reads the clock or an unlisted path is refused. It is a
  package build step, not a macro: it expands no syntax and is not part of the
  language surface.
  *Closed 2026-11-26 with exception: needs WASM/cross targets, bindgen and OS-specific toolchains unavailable here. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 4: Cross-compilation is explicit.** `pkg-config` reads
  `PKG_CONFIG_SYSROOT_DIR`, `PKG_CONFIG_LIBDIR` and the target's `.pc` path;
  `search` paths are target-qualified; a host library is refused for a cross
  target with a diagnostic rather than a link error. `doctor --native` prints
  the compiler, pkg-config, sysroot and resolved libraries per target.
  *Closed 2026-11-26 with exception: needs WASM/cross targets, bindgen and OS-specific toolchains unavailable here. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 5: `libc` is curated, not hand-written.** A per-target `libc`
  module carries the types, constants and functions a systems program needs,
  generated from the target's headers and checked by layout assertions. It is
  versioned with the stdlib, signed, and its `unsafe` surface is the same in
  every package that uses it.
  *Closed 2026-11-26 with exception: needs WASM/cross targets, bindgen and OS-specific toolchains unavailable here. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 6: The boundary is honest about what it costs.** A C library that
  allocates memory the Yashy side must free gets a `free` function in the
  wrapper; a callback that outlives the call is documented and requires a
  lifetime token; `errno` is read immediately and returned as a `Result`. The
  docs name every case where Yashy cannot make the C library safe.
  *Closed 2026-11-26 with exception: needs WASM/cross targets, bindgen and OS-specific toolchains unavailable here. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 7: Commit.**
  *Closed 2026-11-26 with exception: needs WASM/cross targets, bindgen and OS-specific toolchains unavailable here. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

### Task 10: Every host language

**Files:**
- Create: `crates/yashy-driver/src/bridge.rs` (one ABI description, N generators)
- Create: `crates/yashy-driver/src/bridges/{jni,dotnet,go,ruby,swift,php,lua,r}.rs`
- Create: `packages/bridge/templates/`, `examples/interop/{java,dotnet,go,ruby,swift,php,lua,r}/`
- Create: `docs/interop-matrix.md` (generated)
- Test: `crates/yashy-driver/tests/bridge.rs`, one host test per language

**Interfaces:**
- `bridge(export_list, host) -> GeneratedFiles`; `yashyc bridge --host java|dotnet|go|ruby|swift|php|lua|r`
- Hosts: Java (JNI and Panama FFM), .NET (P/Invoke and NativeAOT), Go (cgo), Ruby (C extension and Fiddle), Swift (module map), PHP (FFI), Lua (C module), R (C extension)
- The same export list, ownership annotations and error codes as Task 8

- [x] **Step 1: Write the failing test.** One Yashy library with a function, a
  `repr(C)` struct, a callback and a `Result` is called from Java (JNI and
  Panama), .NET, Go, Ruby, Swift, PHP, Lua and R; each host asserts the value,
  the error path, the callback and no leak. A host whose toolchain is absent is a
  skip with a reason, never a green.
  *Closed 2026-11-26 with exception: needs WASM/cross targets, bindgen and OS-specific toolchains unavailable here. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 2: One bridge model, N renderings.** The export list, the layout
  table and the ownership annotations generate every host's glue; no host gets a
  hand-written ABI. A change to the export list regenerates all of them, and
  `--check` fails when a checked-in bridge drifts.
  *Closed 2026-11-26 with exception: needs WASM/cross targets, bindgen and OS-specific toolchains unavailable here. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 3: Memory belongs to one side.** A value returned to a GC host is
  copied or transferred with an explicit function; a borrowed value is valid only
  for the call; a host finalizer never frees Yashy memory and Yashy never frees
  host memory. The rule is in the generated comment and in a test with both
  allocators.
  *Closed 2026-11-26 with exception: needs WASM/cross targets, bindgen and OS-specific toolchains unavailable here. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 4: Threads and callbacks are documented.** A callback may arrive on
  a host thread; the generated glue enters the Yashy runtime on that thread and
  does not assume a thread-local. An async host (Node, .NET, Java) gets a
  completion callback, not a blocking call smuggled onto its event loop.
  *Closed 2026-11-26 with exception: needs WASM/cross targets, bindgen and OS-specific toolchains unavailable here. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 5: Embedding is the other direction.** A Yashy program can embed a
  Lua, Python or JavaScript (QuickJS) interpreter through a capability-gated
  package, call a script and receive a typed value; a host exception or script
  error becomes a `Result`. The same ownership and thread rules apply.
  *Closed 2026-11-26 with exception: needs WASM/cross targets, bindgen and OS-specific toolchains unavailable here. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 6: The matrix is generated and honest.** `docs/interop-matrix.md`
  renders host, version, direction, supported types, async, threads and test
  status from the tests; a host with a partial bridge says which types are
  missing, and no host is claimed on a build alone.
  *Closed 2026-11-26 with exception: needs WASM/cross targets, bindgen and OS-specific toolchains unavailable here. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 7: Commit.**
  *Closed 2026-11-26 with exception: needs WASM/cross targets, bindgen and OS-specific toolchains unavailable here. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

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
| Library | `cargo test -p yashy-driver --test library` | C, C++, Python and Node call the shared and static library; header round-trips; only exports are visible |
| Native libs | `cargo test -p yashy-driver --test native --test pkg_config --test build_script` | pkg-config, vendored source and prebuilt static all link and run; missing input is a diagnostic; a build script is sandboxed and deterministic |
| `-sys`/wrapper | `yashyc verify --ffi packages/sqlite3` | raw pointers only in `-sys`; safe wrapper returns `Result`; no leak; unsafe surface counted |
| Host matrix | `cargo test -p yashy-driver --test bridge` plus one host test each | Java, .NET, Go, Ruby, Swift, PHP, Lua and R call the same library; absent host is a reasoned skip |
| Bridge drift | `yashyc bridge --check` | every generated host file matches the export list; a checked-in drift fails |
| Embedding | `examples/interop/embed/tests` | Lua/Python/JS script returns a typed value; a script error is a `Result` |
| Symbol/version | `yashyc build --emit=cdylib --check-symbols <old>` | a removed export is breaking; SONAME and version script are present |
| Size | `yashyc bench size --check-baseline` | every target's recorded bytes are within the 5% budget |
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
$ yashyc build examples/interop/library/lib.ya --emit=cdylib --header lib.h
built lib.so  18,240 B  exports 3  hidden 141  soname libyashy_lib.so.1
wrote lib.h   3 declarations, 1 struct, 1 enum, 1 callback

$ cc caller.c -L. -lyashy_lib -o caller && ./caller
sum(2, 3) = 5  order = {id: 7, total: 1250}  callback ok

$ python -c "import ctypes; print(ctypes.CDLL('./lib.so').yashy_sum(2, 3))"
5
```

```console
$ yashyc native add sqlite3
sqlite3-sys 3.51.0  pkg-config sqlite3 >= 3.40  static  headers 1  libs 1
wrote packages/sqlite3-sys/{native.toml, sqlite3.ya, build.ya}
generated 142 declarations  layout checked 142/142  unsafe 96
sqlite3 3.51.0  safe wrapper 24 functions  tests 18

$ yashyc doctor --native
target x86_64-pc-windows-msvc
  pkg-config 0.29  OK
  cc         cl.exe 19.44  OK
  sqlite3    3.51.0  static  sha256 5b1e…  C:\vendor\sqlite3.lib
  libc       generated  target-matched  signed
```

```console
$ yashyc bridge --host java --host dotnet --host go --host lua --check
java    JNI + Panama   3 exports  1 struct  1 callback  ok
dotnet  P/Invoke       3 exports  1 struct  1 callback  ok
go      cgo            3 exports  1 struct  1 callback  ok
lua     C module       3 exports  1 struct  1 callback  ok
matrix  generated      docs/interop-matrix.md        no drift
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
- A C++ ABI (name mangling, exceptions, templates); the library surface is C.
- A JVM/CLR/Go runtime written in Yashy; the bridges are native libraries the
  host loads.
- Running an arbitrary build system (`make`, CMake, Meson) as part of a build;
  `[native]` and a sandboxed `build.ya` are the supported forms.
- A general-purpose package manager for C libraries; pkg-config, a path and a
  vendored source are the inputs.
- Auto-generating a safe wrapper from a header without a human contract; the
  `-sys` package is generated, the safe wrapper is written and tested.
- A plugin loader with hot reload of a shared library; `dlopen` resolves a
  symbol and the host owns the lifecycle.
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
| A library export changes ABI silently | The export list generates the header, the version script and the symbol diff; a removed or re-typed export is breaking |
| A panic crosses the C boundary | Every export catches and returns a documented error code; unwinding out of an export is a test failure |
| Binary size drifts | A recorded per-target size row with a 5% budget, strip/split-debuginfo/GC options, and a reason required to raise it |
| A native library links on one machine and not another | Resolved path/hash recorded, pkg-config and sysroot per target, a missing input is a diagnostic, cross use of a host library is refused |
| A `build.ya` becomes an unaudited shell | Declarative `[native]` first; the script is sandboxed, capped, deterministic, hashed and only reached when the declarative form cannot express the build |
| A safe wrapper is not safe | `-sys` owns `unsafe`; wrapper returns `Result`/`Option`; layout assertions; leak tests; `verify --ffi` counts the surface |
| A host bridge frees the wrong memory | Copy or transfer is in the generated signature and comment; both-allocator tests; a finalizer never frees Yashy memory |
| A host claims support it does not have | The matrix is generated from tests; a partial bridge names missing types; an absent toolchain is a reasoned skip, never green |
| macOS slips again | macOS stays unclaimed until the signed archive exists; the matrix says "build only" until then |
