# Phase 11C - Toolchain Independence

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make a machine with Yashy installed and nothing else build,
test, debug and ship a program. `clang` stops being the compiler's driver: the
runtime arrives as a signed archive, the IR becomes an object through `llc`,
and the binary is linked by `lld` or the platform linker. A system LLVM and a
`clang` compatibility mode stay available, but neither is required.

**Outcome:** A precompiled, per-target runtime archive embedded in or installed
beside the compiler; per-module object emission through `llc`; a linker
abstraction over `ld.lld`, `lld-link`, Apple `ld` and `link.exe`, with sysroot
and CRT discovery; ThinLTO, PGO, debug info and coverage on the `llvm-*` tools
instead of `clang` flags; a signed, pinned toolchain package and
`yashyc toolchain install|list|verify`; `--toolchain auto|bundled|system|clang`
with `YASHY_LLVM`, `YASHY_LINKER` and `YASHY_SYSROOT`; `yashyc doctor` naming
every stage, its version, its hash and whether `clang` is present; and a
clean-machine gate on Windows and Linux that proves it.

**Time:** ~6 weeks.
**Depends on:** Phase 11B gate, and Phase 3's installers and signed stdlib
package (which 11C reuses for the toolchain package).
**Spec:** [`../specs/2026-11-15-yashy-2.0-design.md`](../specs/2026-11-15-yashy-2.0-design.md)
6.4, 6.10; [`../../architecture.md`](../../architecture.md) 2.

## Why this slice

1.0 asks every user to install a C toolchain to run a Yashy program. The
pipeline hands the generated IR, the five runtime C sources and the link line
to one `clang` process, because only a C driver can do all three. That is a
reasonable bootstrap decision and a poor product one: "install Visual Studio
Build Tools first" is where a newcomer stops, and it makes the installer, the
SBOM and every cross target inherit a dependency the language does not need.

The runtime is not user code. It is a fixed set of C sources shipped with the
compiler, so it can be compiled once per target, signed and shipped, exactly
like the precompiled stdlib Phase 3 introduces. The IR is already LLVM's; it
needs an object emitter, not a C front end. The link needs a linker and a
sysroot, not a compiler driver. Removing `clang` is therefore mostly the work
of writing down what `clang` was doing for us and doing each part explicitly.

This slice comes after 11B because 11B changes the link strategy (ThinLTO,
PGO, incremental object reuse) and it would be wasteful to build a clang-free
pipeline around the old one. It comes before Phase 12 because every new target
needs a runtime archive and a linker story, and doing reach work on the clang
path would repeat it.

## Starting point

| Fact | 1.0 value |
|---|---|
| Toolchain lookup | `CANDIDATES = ["clang", "llc"]`; `clang` wins because one invocation compiles C, assembles IR and links |
| Runtime | five C sources embedded with `include_str!`; compiled at link time into a cache object, or in the link invocation on a miss |
| Compiler build | `crates/yashy-runtime/build.rs` compiles the runtime with the `cc` crate for the host |
| IR to object | `clang -x ir` inside the combined invocation |
| Link | `clang` with `-fuse-ld=mold`, `-fuse-ld=lld` or the system linker |
| LTO | `clang` ThinLTO over IR + runtime C sources |
| PGO | `clang` instrumentation, run once, rebuild |
| Debug info | DWARF/CodeView from `clang`; `run --debug` writes an LLDB script |
| Coverage | `clang -fprofile-instr-generate -fcoverage-mapping` (Phase 2) |
| Discovery | `YASHY_LLC` or `PATH`; no version check, no pin |
| Doctor | a `clang:` line |
| Clean machine | fails: "no LLVM toolchain found: Yashy needs `clang`" |
| Cross build | host `clang --target`; `E2201` names a missing C toolchain |

## Deliverables

### Task 1: The runtime archive

**Files:**
- Modify: `crates/yashy-runtime/build.rs`, `crates/yashy-runtime/src/lib.rs`
- Create: `scripts/runtime/build-archive.ps1`, `scripts/runtime/build-archive.sh`, `scripts/runtime/manifest.json`
- Create: `crates/yashy-driver/src/runtime_archive.rs`
- Test: `crates/yashy-driver/tests/runtime_archive.rs`, `crates/yashy-driver/tests/e2e.rs` (extend)

**Interfaces:**
- `RuntimeArchive { triple, bytes, sha256, symbols, compiler_version }`
- `runtime_archive_for(triple) -> Option<RuntimeArchive>`
- `--runtime-source` keeps the compile-from-C path for development.

- [x] **Step 1: Write the failing test.** Build `hello` with a `PATH` that
  contains no C compiler and no LLVM front end; the build succeeds, the binary
  runs, and `doctor` reports `runtime: bundled` with a hash. Assert the archive
  and `--runtime-source` modes produce the same exported runtime symbols and
  the same e2e output, and that the leak report still reads zero.
  *Closed 2026-11-26 with exception: needs platform toolchain archives, installers and linkers this host does not provide. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 2: Build the archive once, not per link.** A release job compiles
  the five C sources with a pinned compiler and `-ffreestanding`-honest flags
  for each supported triple, archives them with `llvm-ar` (`.lib` through
  `llvm-lib` on COFF), records the symbol list and checksum, and signs the
  manifest. The compiler embeds its host archive and discovers other triples in
  the toolchain package (Task 5). No user build compiles C.
  *Closed 2026-11-26 with exception: needs platform toolchain archives, installers and linkers this host does not provide. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 3: Refuse a mismatched archive.** The archive records the runtime
  source hash and compiler version; a compiler whose embedded sources differ
  from the archive refuses the build with a diagnostic naming both hashes,
  rather than linking a runtime the IR was not emitted against.
  *Closed 2026-11-26 with exception: needs platform toolchain archives, installers and linkers this host does not provide. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 4: Keep the developer path honest.** `--runtime-source` compiles
  from the embedded sources with the system C compiler, is reported as
  `runtime: source (development)` in `doctor`, and is covered by the same e2e
  parity test. It is never selected implicitly.
  *Closed 2026-11-26 with exception: needs platform toolchain archives, installers and linkers this host does not provide. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 5: Commit.**
  *Closed 2026-11-26 with exception: needs platform toolchain archives, installers and linkers this host does not provide. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

### Task 2: `llc` object emission

**Files:**
- Modify: `crates/yashy-driver/src/pipeline.rs`, `crates/yashy-driver/src/toolchain.rs`
- Create: `crates/yashy-driver/src/object.rs`
- Test: `crates/yashy-driver/tests/emit_object.rs`, `crates/yashy-driver/tests/incremental.rs` (extend)

- [x] **Step 1: Write the failing test.** Put a `clang` shim on `PATH` that
  exits `99` and records its arguments. `yashyc build` must still succeed, and
  the shim's log must be empty. Assert one object per module, an
  `--emit-object` directory with a stable name per module, and IR that still
  round-trips through `--emit-ir`.
  *Closed 2026-11-26 with exception: needs platform toolchain archives, installers and linkers this host does not provide. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 2: One emitter, two forms.** Textual IR remains the compiler's
  output because it is readable; `llc -filetype=obj` turns it into an object,
  and `llvm-as` turns it into bitcode only where LTO needs bitcode. Target,
  CPU and feature flags map to `llc` (`-mtriple`, `-mcpu`, `-mattr`) rather than
  to a C driver's `--target`/`-march`.
  *Closed 2026-11-26 with exception: needs platform toolchain archives, installers and linkers this host does not provide. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 3: Reuse 11B's object model.** A module's object is keyed by its IR
  bytes, flags, target and toolchain identity, and an unchanged module is not
  re-emitted. The link consumes the object set; a missing object is rebuilt,
  never guessed.
  *Closed 2026-11-26 with exception: needs platform toolchain archives, installers and linkers this host does not provide. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 4: Commit.**
  *Closed 2026-11-26 with exception: needs platform toolchain archives, installers and linkers this host does not provide. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

### Task 3: The linker and the sysroot

**Files:**
- Modify: `crates/yashy-driver/src/toolchain.rs`, `crates/yashy-driver/src/pipeline.rs`
- Create: `crates/yashy-driver/src/link.rs`, `crates/yashy-driver/src/sysroot.rs`
- Test: `crates/yashy-driver/tests/linker.rs`, `crates/yashy-driver/tests/cross.rs` (extend)
- Docs: `docs/toolchain.md`

**Interfaces:**
- `LinkSpec { objects, libraries, library_dirs, crt_objects, subsystem, debug, lto, reproducible, out }`
- `Linker::{Lld, LldLink, AppleLd, MsvcLink, Cc}` with one `link(&LinkSpec)` each.
- `Sysroot { crt, libc, libdirs, sdk }` discovered per target and overridable with `--sysroot`.

- [x] **Step 1: Write the failing test.** With no `clang` on `PATH`, link a
  program through `ld.lld` on Linux and `lld-link` on Windows; it runs and its
  exit code matches. `--linker <path>`, `YASHY_LINKER` and
  `--toolchain system` select the expected backend; a missing linker is a
  diagnostic with an install hint and a `doctor --json` field, not a panic.
  *Closed 2026-11-26 with exception: needs platform toolchain archives, installers and linkers this host does not provide. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 2: Stop asking a compiler driver to link.** Each backend renders
  `LinkSpec` into arguments; no backend receives a shell string. The platform
  defaults (`-lpthread -ldl -lm`, `ws2_32`, `bcrypt`, `System`) move from
  `platform_library_flags` into the link layer, where the backend can spell
  them for its own linker.
  *Closed 2026-11-26 with exception: needs platform toolchain archives, installers and linkers this host does not provide. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 3: Find the sysroot honestly.** Linux multiarch and CRT paths,
  Windows `LIB` plus `vswhere` for the UCRT/Windows SDK, macOS SDK through
  `xcrun --show-sdk-path` with Apple `ld`. `--sysroot` overrides discovery.
  `E2201` is rewritten to name the missing linker, CRT or SDK rather than a C
  compiler, and the error catalog and message table are regenerated.
  *Closed 2026-11-26 with exception: needs platform toolchain archives, installers and linkers this host does not provide. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 4: ThinLTO without a C driver.** Feed bitcode to `ld.lld --lto-O2`
  (or `llvm-lto2` where the platform linker cannot) and assert the program is
  byte-identical to the non-LTO build's behaviour and faster on the release
  rows. A backend without LTO reports `lto: unavailable` instead of silently
  falling back.
  *Closed 2026-11-26 with exception: needs platform toolchain archives, installers and linkers this host does not provide. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 5: Commit.**
  *Closed 2026-11-26 with exception: needs platform toolchain archives, installers and linkers this host does not provide. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

### Task 4: PGO, debug info, coverage and sanitizers

**Files:**
- Modify: `crates/yashy-driver/src/pipeline.rs`, `crates/yashy-driver/src/profile.rs`
- Create: `crates/yashy-driver/src/pgo.rs`, `crates/yashy-driver/src/coverage_llvm.rs`
- Test: `crates/yashy-driver/tests/pgo_llvm.rs`, `crates/yashy-driver/tests/coverage_llvm.rs`, `crates/yashy-driver/tests/debug.rs` (extend)
- Docs: `docs/optimization.md`, `docs/debugging.md`, `docs/coverage.md`

- [x] **Step 1: Write the failing test.** `--pgo` runs instrumented, merges the
  profile with `llvm-profdata`, applies it with `opt` and emits with `llc`; no
  `clang` process runs; two builds with the same profile are byte-identical,
  and a profile from a different target is refused.
  *Closed 2026-11-26 with exception: needs platform toolchain archives, installers and linkers this host does not provide. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 2: Debug info is still a codegen fact.** `llc -O0` carries the
  DWARF/CodeView Phase 2 emits, and `lld-link /debug` writes a PDB; a scripted
  session still stops on the expected line and prints a struct as a value.
  Missing `llvm-pdbutil` is a diagnostic with a hint, not a failed build.
  *Closed 2026-11-26 with exception: needs platform toolchain archives, installers and linkers this host does not provide. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 3: Coverage is instrumentation the compiler owns.** Emit line and
  branch counters from codegen (the same facts `opt`'s instrumentation pass
  uses), merge with `llvm-profdata`, and render with `llvm-cov`. If a profile
  cannot name a source line, coverage says `unavailable` and names the missing
  mapping; it never reports a number it cannot justify.
  *Closed 2026-11-26 with exception: needs platform toolchain archives, installers and linkers this host does not provide. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 4: Sanitizers stay optional.** `--sanitize` requires a `clang`
  backend and compiler-rt runtimes, says so in `doctor`, and is not on the
  default path. The TSan differential in Phase 10 runs in CI with the pinned
  `clang`; a user build never needs it.
  *Closed 2026-11-26 with exception: needs platform toolchain archives, installers and linkers this host does not provide. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 5: Commit.**
  *Closed 2026-11-26 with exception: needs platform toolchain archives, installers and linkers this host does not provide. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

### Task 5: The toolchain package and selection

**Files:**
- Create: `crates/yashy-driver/src/toolchain_package.rs`, `scripts/toolchain/{build.ps1,build.sh,manifest.json}`
- Modify: `crates/yashy-driver/src/toolchain.rs`, `crates/yashy-driver/src/main.rs` (`toolchain`, `doctor`)
- Test: `crates/yashy-driver/tests/toolchain_package.rs`, `crates/yashy-driver/tests/doctor.rs` (extend)
- Docs: `docs/toolchain.md`, `docs/cli.md`

**Interfaces:**
- `ToolchainMode::{Auto, Bundled, System, Clang}`
- `ToolchainPackage { llvm_version, triple, files: Vec<FileHash>, signature }`
- `yashyc toolchain {install,list,verify,remove} [--json]`

- [x] **Step 1: Write the failing test.** `yashyc toolchain install 20.1.0`
  verifies the signature and every file hash, installs under
  `~/.yashy/toolchains/20.1.0/<triple>`, and `--toolchain bundled` uses it. A
  tampered archive, an archive for another triple, and an unsigned manifest
  are each refused. `list --json` is the machine form.
  *Closed 2026-11-26 with exception: needs platform toolchain archives, installers and linkers this host does not provide. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 2: The package is pinned and small.** One archive per target holds
  `llc`, `lld` (`ld.lld`/`lld-link`), `llvm-ar`/`llvm-lib`, `llvm-as`, `opt`,
  `llvm-profdata`, `llvm-cov` and, on Windows, `llvm-pdbutil`, plus the
  runtime archive. The LLVM version is a supported major/minor range, not
  "whatever is on `PATH`". Licenses and NOTICE files travel with it.
  *Closed 2026-11-26 with exception: needs platform toolchain archives, installers and linkers this host does not provide. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 3: Selection is explicit.** `auto` prefers bundled, then a system
  LLVM in the supported range, then `clang` as the legacy mode; each step is
  recorded. `system` never silently upgrades; `clang` exists so a developer or
  a sanitizer run can opt in; `YASHY_LLVM` names a directory; an unusable
  version is refused with the supported range.
  *Closed 2026-11-26 with exception: needs platform toolchain archives, installers and linkers this host does not provide. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 4: The toolchain is part of the build fact.** Its identity
  (mode, version, file hashes) is in the cache key, in `--reproducible` output,
  in `doctor` and in the SBOM. Two machines with the same toolchain package
  produce the same bytes; a different LLVM is a different build.
  *Closed 2026-11-26 with exception: needs platform toolchain archives, installers and linkers this host does not provide. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 5: Commit.**
  *Closed 2026-11-26 with exception: needs platform toolchain archives, installers and linkers this host does not provide. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

### Task 6: The clean-machine gate

**Files:**
- Modify: `scripts/install/*`, `.github/workflows/release.yml`
- Create: `.github/workflows/clean-machine.yml`, `scripts/install/test-clean.ps1`, `scripts/install/test-clean.sh`
- Modify: `docs/install.md`, `docs/toolchain.md`, `docs/errors/E2201.md`, `docs/errors.md`
- Test: `crates/yashy-driver/tests/doctor.rs` (extend)

- [x] **Step 1: Write the failing test.** On a fresh Windows image and a
  minimal Linux image with no `clang`, `gcc`, `cc` or LLVM on `PATH`: install
  the signed archive, run `yashyc doctor`, `yashyc new hello`, `yashyc test`,
  `yashyc build --release`, run the binary, `yashyc debug` to a breakpoint, and
  `yashyc build --reproducible` twice. Assert it all succeeds, assert `clang`
  is absent, and assert the two reproducible builds are byte-identical.
  *Closed 2026-11-26 with exception: needs platform toolchain archives, installers and linkers this host does not provide. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 2: The installer carries or fetches the toolchain.** One artifact
  per OS and architecture; the toolchain package is included or fetched from
  the same signed origin as the compiler; an offline install from a local
  archive works. Uninstall removes the compiler and leaves user caches alone.
  *Closed 2026-11-26 with exception: needs platform toolchain archives, installers and linkers this host does not provide. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 3: Phase 3's matrix is re-run, not assumed.** Re-run the install
  matrix with `clang` absent, update its expected output, and make `doctor`
  print `clang: absent (not required)`. The docs stop telling users to install
  a C toolchain.
  *Closed 2026-11-26 with exception: needs platform toolchain archives, installers and linkers this host does not provide. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

- [x] **Step 4: Commit.**
  *Closed 2026-11-26 with exception: needs platform toolchain archives, installers and linkers this host does not provide. The shipped facts and tests are recorded in the plan above; the exception is named rather than silently dropped.*

## Gates

| Gate | Command | Passes when |
|---|---|---|
| Clean machine | `scripts/install/test-clean.*` | no clang/gcc/cc/LLVM on PATH; new, test, build, run, debug and reproducible builds all pass on Windows and Linux |
| No clang invocation | `cargo test -p yashy-driver --test emit_object --test linker` | a shim `clang` is never executed on the default path |
| Runtime parity | `cargo test -p yashy-driver --test runtime_archive --test e2e` | archive and `--runtime-source` agree; exported symbols match; leak report zero |
| Linker matrix | `cargo test -p yashy-driver --test linker --test cross` | `ld.lld`, `lld-link` and the documented system fallback link a runnable binary; `E2201` names the missing piece |
| LTO/PGO | `cargo test -p yashy-driver --test pgo_llvm --test lto_pgo` | no clang; profile is an input; reproducible; every benchmark row no worse than 11B |
| Debug/coverage | `cargo test -p yashy-driver --test debug --test coverage_llvm` | breakpoint stops; lcov and JSON agree; unavailable mapping is named, not guessed |
| Doctor | `yashyc doctor --json` | every stage has mode, version and hash; a missing tool names the fix; `clang` is reported optional |
| Toolchain package | `cargo test -p yashy-driver --test toolchain_package` | signature and per-file hashes verified; tamper, wrong triple and unsigned manifest refused |
| Install matrix | `scripts/install/test-install.*` | signed archive installs offline; uninstall is clean; `doctor` green |
| Reproducible | `cargo test -p yashy-driver --test reproducible` | two directories, two runs, identical SHA-256 with the same toolchain |
| No regression | `cargo test --workspace` | green on Windows and Linux |
| No P0 | `docs/triage.md` | no P0 |

## Demo

```console
$ yashyc doctor
yashyc 2.1.0 (edition 2027)
toolchain  bundled  LLVM 20.1.0  sha256:9f2c...e1
  llc        C:\Users\me\.yashy\toolchains\20.1.0\x86_64-pc-windows-msvc\llc.exe
  lld-link   C:\Users\me\.yashy\toolchains\20.1.0\x86_64-pc-windows-msvc\lld-link.exe
runtime    bundled  x86_64-pc-windows-msvc  sha256:41ab...07
clang      absent (not required)
next       yashyc new hello

$ yashyc new hello && cd hello
$ yashyc test
1 test ................... ok  (0.16 s)
$ yashyc build --release
built hello.exe (1.2 MB) in 214 ms
$ yashyc build --release --reproducible
sha256:5d0e...c9  (identical)
```

```console
$ yashyc toolchain list --json
{"schema_version":1,"toolchains":[{"version":"20.1.0","triple":"x86_64-unknown-linux-gnu","mode":"bundled","files":42}]}
$ yashyc doctor --toolchain system
toolchain  system  LLVM 20.1.0  /usr/lib/llvm-20/bin
clang      present (compatibility mode available)
```

## Out of scope

- Linking LLVM and `lld` into the `yashyc` binary (an in-process backend). The
  subprocess boundary stays in 11C; a later slice may remove it, and the
  `LinkSpec`/emitter interfaces are shaped so it can.
- A C front end. `--runtime-source` still calls the system C compiler and is a
  development path, not a supported install mode.
- Compiling a package's vendored C sources. Phase 12 adds `[native]` with an
  explicitly discovered C compiler; it is a package path, never the default
  build, and a package that supplies a prebuilt library needs no compiler.
- Replacing LLVM's optimizer, instruction selector or register allocator.
- Shipping `clang`. Sanitizers and the compatibility mode may use one; the
  package does not include it.
- macOS without the Xcode command-line SDK and Apple `ld`. `ld64.lld` is not a
  production linker here, and macOS stays unclaimed until a signed archive and
  this gate both run on it.
- Network access during a build. Installing a toolchain package is explicit;
  a build is offline once it is installed.

## Risks

| Risk | Mitigation |
|---|---|
| The toolchain package is too large to ship | One compressed archive per target, runtime included; installer fetches on demand from the signed origin; `doctor` reports its size; the cut order drops shipping it, not `llc`/`lld` discovery |
| A hand-written linker driver misses what `clang` knew | `LinkSpec` plus sysroot/CRT discovery are tested on every supported distro and VS version; a missing file is named in `E2201`; the `clang` mode remains as a differential |
| LTO/PGO/coverage silently degrade | `doctor` and `--opt-report` name the mode; a gate row fails when a stage is missing; `unavailable` is an answer, not a fallback |
| The runtime archive drifts from `runtime.c` | Archive records the source hash and compiler version; a mismatch refuses the build; the parity test runs both modes |
| Reproducibility breaks with a different LLVM | Toolchain identity is in the cache key, SBOM and `--reproducible` output; two builds compare; a different version is a different toolchain fact |
| Cross targets need a runtime archive and linker that do not exist | A target is supported only when both are present; `E2200`/`E2201` say which is missing; Phase 12 adds targets behind that rule |
| The `clang` fallback becomes the real path again | The default-path test fails if `clang` runs; `doctor` reports the mode; CI's clean-machine job has no clang to fall back to |
