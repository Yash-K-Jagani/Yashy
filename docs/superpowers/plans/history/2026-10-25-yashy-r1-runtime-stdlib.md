# Slice 2 — Runtime Standard-Library Gaps

> **For agentic workers:** Read the master plan and slice 1 first. This slice
> changes the program entry ABI and the runtime C sources. Run the full e2e
> suite on Windows and Linux before calling it done.

**Status:** Implemented and pushed, commit `d30e1af` on `slice-1-install-first-cli`,
2026-10-26. Linux is verified end to end; Windows test execution is delegated to
CI; the `ubuntu:22.04` container acceptance is pending a Docker host.
**Source of truth:** `docs/superpowers/specs/2026-10-25-yashy-super-language-v2-design.md` §15.2.
**Master plan:** `docs/superpowers/plans/2026-10-25-yashy-super-language-master.md` §7.2.
**Goal:** Implement `args()`, `getcwd()`, `now()`, and `sleep(ms)` as real
builtins, with runtime support, ownership, leak tests, and updated docs.
**Depends on:** Slice 1 for the first-CLI guide update; the runtime work itself
is independent.
**Estimate:** 8–10 working days.
**Decisions confirmed (2026-10-25):** `args()` excludes the program path;
`getcwd()` returns `""` on failure; Windows arguments use the C `char**`
(code-page) encoding while `getcwd()` returns UTF-8; builtin `Result` is R2.

---

## 1. Scope

### In scope

- A hidden `argc`/`argv` at the LLVM entry point, with `yashy_args_init`.
- Runtime C entry points:
  - `yashy_args_slice` for `args()`.
  - `yashy_getcwd` for `getcwd()`.
  - `yashy_now_ms` for `now()`.
  - `yashy_sleep_ms` for `sleep(ms)`.
- Builtin table entries and codegen lowering.
- Stdlib documentation for the four names.
- A test that bundled stdlib discovery works with `YASHY_PATH` unset.
- Tests: runtime C, codegen, e2e, leaks, and the first-CLI guide.
- The "add arguments" section in the first-CLI guide.

### Not in scope

- `Result`-returning builtin signatures. R1 keeps the existing
  empty-string/0 convention for `getcwd()` and records the exception; builtin
  `Result` is R2 work.
- Unicode argument handling on Windows beyond UTF-8 conversion for `getcwd()`.
  Arguments use the process's `char**` encoding in R1; the limitation is
  documented and revisited with Unicode strings in 1.1.
- Environment mutation, process spawning, monotonic clocks, time zones, or
  formatting.
- Async or timers.

---

## 2. Current state

- `main` is emitted as `i32 @main()` by `gen_main`/`gen_fn_impl` in
  `crates/yashy-codegen/src/fn_gen.rs`. The Yashy-level `fn main()` takes no
  parameters; the checker rejects parameters on `main`.
- The runtime C is embedded via `RUNTIME_C_SOURCE` in
  `crates/yashy-runtime/src/lib.rs`; the driver writes it to a cache and
  compiles it once per toolchain. `runtime_cache_key` hashes the C source and
  header, so changing them invalidates the cache automatically.
- The builtin table is a fixed array in
  `crates/yashy-sema/src/check_expr.rs` (currently 26 entries). Builtins are
  predeclared in the root scope and calls resolve through the normal call path.
- Builtin lowering is in `crates/yashy-codegen/src/expr_gen.rs` around the
  `match name.as_str()` block; string-returning C builtins use
  `gen_cstr_string_builtin` or `gen_borrowed_call`.
- Runtime declarations are emitted up front in
  `crates/yashy-codegen/src/module.rs`.
- `args.ya` and `time.ya` document the names as unavailable; `os.ya` documents
  `getcwd()` as unavailable.
- `yashy run` already forwards `-- <args>` to the child, but the child cannot
  read them.

---

## 3. Design decisions

### 3.1 Entry point

Emit the C entry signature directly:

```llvm
define i32 @main(i32 %argc, i8** %argv) {
  call void @yashy_args_init(i32 %argc, i8** %argv)
  ; ... the Yashy main body ...
}
```

The Yashy surface stays `fn main()`; the two hidden parameters exist only in
the LLVM signature and are not visible to the checker. This avoids a wrapper
function, keeps recursive `main` calls resolving to the same symbol, and keeps
the existing `i32` return/truncation rules.

The alternative — a wrapper `@main(i32, i8**)` calling a renamed user main —
was rejected: it breaks the "`main` keeps its own name" rule and adds a second
function for every program.

**Cost:** every program carries two unused parameters and one call at entry.
That is negligible compared with process startup, and it is the only portable
way to get the real argument vector.

### 3.2 `args()` semantics

- `args()` returns the **user arguments**, excluding the executable path, so
  `args()[0]` is the first argument after `yashy run --`.
- The result is an owned `[]string`. Each element is an owned string. The
  caller releases the slice with `.free()`, the same method `split`/`lines`
  results use; there is no `free_slice` builtin.
- An empty argument vector returns an empty slice, not an error.
- The program name is available internally but is not exposed in R1. If a
  future `program_name()` is wanted, it is a separate builtin.

The alternative — include the program name like C `argv` — is defensible but
makes every CLI write `args()[1]` for the first user argument. Yashy's builtin
is a convenience, so exclude it and document the choice.

### 3.3 `getcwd()`

- `getcwd()` returns the current working directory as an owned string.
- On failure it returns the empty string, matching `read_file`/`getenv` in R1.
  This is the recorded exception to the spec's "fallible operations return
  `Result`" claim; builtin `Result` is R2.
- POSIX: `getcwd` with a growing buffer; retry on `ERANGE`.
- Windows: `GetCurrentDirectoryW` plus `WideCharToMultiByte(CP_UTF8)`, so the
  path is UTF-8 like the rest of Yashy's byte strings.

### 3.4 `now()` and `sleep(ms)`

- `now()` returns milliseconds since the Unix epoch as an `int`. It is a wall
  clock: it can jump backwards if the system clock changes. Document that.
- `sleep(ms)` blocks for at least `ms` milliseconds, where `ms <= 0` is a
  no-op. It returns unit.
- POSIX: `nanosleep` with an `EINTR` retry loop.
- Windows: `Sleep` in a loop for values above `DWORD`'s comfortable range;
  `Sleep(0)` yields the scheduler.
- `now()` uses `timespec_get(&ts, TIME_UTC)` where available. If a target lacks
  it, fall back to `clock_gettime(CLOCK_REALTIME)` or
  `GetSystemTimeAsFileTime`.

### 3.5 Builtin signatures

Add to the table in `check_expr.rs`:

| Name | Parameters | Result | Notes |
| --- | --- | --- | --- |
| `args` | none | `[]string` | Owned slice of owned strings. |
| `getcwd` | none | `string` | Owned; empty on failure. |
| `now` | none | `int` | Milliseconds since the Unix epoch. |
| `sleep` | `int` | `void` | `ms <= 0` is a no-op. |

The array length changes from 26 to 30. The `[]string` type is the same one
`split`/`lines` return; mirror that constructor rather than inventing a new
type spelling.

### 3.6 Runtime API

Add to `yashy_runtime.h` and `runtime.c`:

```c
void yashy_args_init(int argc, char **argv);
void yashy_args_slice(yashy_slice *out);
yashy_str *yashy_getcwd(void);
long long yashy_now_ms(void);
void yashy_sleep_ms(long long ms);
```

`yashy_args_init` stores the vector in process-global state. It is called once
at entry; a second call keeps the first vector. `yashy_args_slice` fills a
fresh slice with owned strings for `argv[1..argc]`. If a push fails, it
releases the string it could not append rather than leaking it.

The runtime's string constructor is static in `runtime.c`; the new functions
live in the same translation unit so they can use it. Do not duplicate the
string layout.

---

## 4. Tasks

### Task 1 — Runtime tests first

- [x] Add `crates/yashy-runtime/tests/args.rs`:
  - call `yashy_args_init` with a synthetic `argv` (`prog`, `one`, `two`);
  - fill a slice with `yashy_args_slice`;
  - assert length 2, contents `one`/`two`;
  - free the slice with `yashy_slice_free_strings`;
  - assert the leak report is zero when the test enables it.
- [x] Add `crates/yashy-runtime/tests/time.rs`:
  - `yashy_now_ms()` is positive and non-decreasing across two calls;
  - `yashy_sleep_ms(0)` returns immediately;
  - `yashy_sleep_ms(30)` takes at least ~20 ms and returns;
  - `yashy_sleep_ms(-1)` is a no-op.
- [x] Add a `getcwd` test that returns a non-empty path containing the current
  test directory's name, and frees the result.

Run them and verify they fail to link/compile before the implementation.

### Task 2 — Runtime implementation

- [x] Add the five declarations to `crates/yashy-runtime/include/yashy_runtime.h`
  with ownership and failure notes.
- [x] Implement them in `crates/yashy-runtime/runtime.c`:
  - includes for `<time.h>` and the platform clock/sleep calls;
  - static `argc`/`argv` storage and `yashy_args_init`;
  - `yashy_args_slice` with owned strings;
  - `yashy_getcwd` with the growing-buffer POSIX path and the UTF-8 Windows
    path;
  - `yashy_now_ms` and `yashy_sleep_ms`.
- [x] Add Rust `extern "C"` declarations in `crates/yashy-runtime/src/lib.rs`
  for the tests. Keep the header and the Rust declarations in sync.
- [ ] Run the runtime tests on Windows and Linux. Linux (WSL) is green; the
  Windows host blocks freshly linked test binaries, so CI's `windows-latest`
  job is the Windows evidence.

### Task 3 — Entry point

- [x] Change `gen_fn_impl` in `crates/yashy-codegen/src/fn_gen.rs`:
  - when `entry_point`, emit `define i32 @main(i32 %argc, i8** %argv)`;
  - emit `call void @yashy_args_init(i32 %argc, i8** %argv)` as the first
    instruction of the entry body;
  - keep the Yashy parameter list, the `i32` return, and the `-> i64`
    truncation unchanged.
- [x] Add `declare void @yashy_args_init(i32, i8**)` in
  `crates/yashy-codegen/src/module.rs`.
- [x] Update every test that asserts `define i32 @main()`:
  - `crates/yashy-codegen/tests/codegen.rs`;
  - `crates/yashy-codegen/tests/externs.rs`;
  - `crates/yashy-driver/tests/e2e.rs`.
  Add a test that `fn main() -> int` still returns the exit status and that a
  recursive `main` still compiles.
- [x] Verify the generated IR with `yashy build --emit-ir` for a hello-world
  and a `-> int` main.

### Task 4 — Builtin table

- [x] Add the four entries to the builtin array in
  `crates/yashy-sema/src/check_expr.rs`; change the array length.
- [x] Add a checker test for each:
  - `args()` has type `[]string`;
  - `getcwd()` has type `string`;
  - `now()` has type `int`;
  - `sleep(10)` has type `void` and `sleep("x")` is a type error;
  - the names cannot be redefined by a user function.
- [x] Confirm `import args; args()` and `import time; now()` resolve by the
  last segment, like `os.exit`.

### Task 5 — Codegen lowering

- [x] Add declarations in `module.rs`:
  - `declare void @yashy_args_slice({ i8*, i64, i64 }*)`
  - `declare i8* @yashy_getcwd()`
  - `declare i64 @yashy_now_ms()`
  - `declare void @yashy_sleep_ms(i64)`
- [x] Lower `getcwd`, `now`, and `sleep` through the existing
  `gen_borrowed_call` path (the `yashy_` prefix is the only difference).
- [x] Lower `args` like `split`/`lines`: allocate a stack `yashy_slice`,
  call `yashy_args_slice`, and return the loaded slice value. Do not build the
  slice element by element in IR.
- [x] Add a codegen test that the four calls emit the expected runtime symbols.

### Task 6 — Stdlib documentation

- [x] Rewrite `crates/yashy-stdlib/yashy/args.ya` to document the builtin:
  excludes the program name, owned `[]string`, free it, empty when there are no
  arguments. Remove the "not provided" text.
- [x] Rewrite `crates/yashy-stdlib/yashy/time.ya` to document `now()` and
  `sleep(ms)`: units, wall-clock caveat, negative sleep, blocking behavior.
- [x] Update `crates/yashy-stdlib/yashy/os.ya`: document `getcwd()` and remove
  the "not provided" paragraph.
- [x] Add a cross-reference in `fs.ya` if it helps discovery.
- [x] Run the every-module stdlib test.

### Task 7 — End-to-end tests

- [x] Add `crates/yashy-driver/tests/programs/args.ya`:
  - print `args().len()`;
  - print each argument;
  - call `.free()` on the slice.
- [x] Add `crates/yashy-driver/tests/programs/cwd.ya`:
  - print `getcwd()`.
- [x] Add `crates/yashy-driver/tests/programs/time.ya`:
  - `let start = now(); sleep(20); let end = now(); assert(end >= start);`
  - print a stable result, not the timestamps.
- [x] Add driver tests:
  - `yashy run args.ya -- one two` prints `2`, `one`, `two`;
  - `yashy run cwd.ya` from a temp directory prints that directory;
  - `yashy run time.ya` exits 0 and takes at least the sleep duration;
  - all three run with `YASHY_LEAK_REPORT=1` and report `0 live allocations`.
- [x] Add the programs to the leak suite and the stdlib/e2e lists as
  appropriate.
- [x] Verify `yashy test` on a project whose test reads arguments? The test
  runner does not pass arguments in R1; do not add that feature here.

### Task 8 — First-CLI guide update

- [x] Add an "Add arguments" section to `docs/tutorial/12-first-cli.md`:
  - replace the stdin-only loop with a file path from `args()`;
  - show `yashy run main.ya -- notes.txt`;
  - show the empty-argument case and a usage message;
  - call `.free()` on the `args()` slice.
- [x] Update the tutorial test to compile and run the argument version with
  `-- notes.txt`.
- [x] Mention that `args()` excludes the program name.

### Task 9 — Docs and spec exception

- [x] Add the R1 exception to the spec's stdlib bullet: the four runtime
  builtins use the existing empty-string/0 convention until builtin `Result`
  lands in R2.
- [x] Update `README.md`'s stdlib paragraph to mention `args`, `time`, and
  `getcwd`.
- [x] Update the stdlib test header and the master plan's gap table.
- [x] Do not claim `Result`-returning `getcwd()`.

### Task 10 — Cache and full-suite verification

- [x] Confirm `runtime_cache_key` changes when `runtime.c`/the header change;
  no manual cache version is needed. If a stale object is observed, bump the
  key or clear the cache in the test and record why.
- [ ] Run the full driver suite on Windows and Linux. Linux (WSL) is green;
  the Windows host blocks freshly linked test binaries, so CI's
  `windows-latest` job is the Windows evidence.
- [x] Run `cargo check --workspace --all-targets`.
- [ ] Verify a clean build (empty `YASHY_CACHE_DIR`) works on both platforms.
  Linux is verified; Windows is delegated to CI for the same host-policy reason.

---

## 5. Verification

```text
# Windows
cargo test -p yashy-runtime
cargo test -p yashy-codegen
cargo test -p yashy-driver --test e2e
cargo test -p yashy-driver --test leaks
cargo test -p yashy-driver --test modules
cargo test -p yashy-driver --test tutorial
cargo check --workspace --all-targets

# Linux (WSL Ubuntu or CI)
CARGO_TARGET_DIR=/tmp/yashy-target-linux cargo test -p yashy-runtime
CARGO_TARGET_DIR=/tmp/yashy-target-linux cargo test -p yashy-codegen
CARGO_TARGET_DIR=/tmp/yashy-target-linux cargo test -p yashy-driver --test e2e
CARGO_TARGET_DIR=/tmp/yashy-target-linux cargo test -p yashy-driver --test leaks
CARGO_TARGET_DIR=/tmp/yashy-target-linux cargo test -p yashy-driver --test modules
CARGO_TARGET_DIR=/tmp/yashy-target-linux cargo test -p yashy-driver --test tutorial
```

Run the suite with `YASHY_PATH` unset on both platforms. The `modules` test
covers bundled discovery; the acceptance run must not depend on `YASHY_PATH`.

The acceptance run:

```text
yashy run crates/yashy-driver/tests/programs/args.ya -- one two
# 2
# one
# two

YASHY_LEAK_REPORT=1 yashy run crates/yashy-driver/tests/programs/args.ya -- one two
# ... 0 live allocations
```

### Verification record (2026-10-26)

- Linux: `cargo test --workspace --no-fail-fast` in WSL Ubuntu 26.04 with
  clang 18 passed, including 113 e2e tests, the runtime `args`/`time` tests,
  the leak suite, the tutorial harness, and the bundled-stdlib discovery test.
- Linux: `cargo check --workspace --all-targets` passed.
- Linux: a clean build with an empty `YASHY_CACHE_DIR` and no `YASHY_PATH`
  built and ran `examples/hello`.
- Windows: `cargo +stable-x86_64-pc-windows-gnu check --workspace --all-targets`
  passed. The host's Application Control policy blocks freshly linked test
  binaries, so the Windows test run is delegated to CI.
- Ubuntu 22.04 / clang 14: the driver now asks `clang --version` and passes
  `-mllvm -opaque-pointers` only for LLVM 14; LLVM 15 made opaque pointers the
  default and LLVM 17 removed the switch. The C runtime sources are compiled
  separately through the cache, so the switch applies to the generated IR.
  The 22.04 container acceptance is still pending because the local Docker
  daemon is unavailable; CI's `ubuntu-22.04` jobs exercise the same path.

---

## 6. Acceptance criteria

- `args()` returns the user arguments on Windows and Linux, excludes the
  program name, and leaks nothing when freed.
- `getcwd()` returns the current directory on both platforms, as UTF-8 on
  Windows.
- `now()` returns a positive millisecond timestamp; `sleep(30)` blocks for at
  least ~20 ms; `sleep(0)` and negative values are no-ops.
- Existing programs still compile and run; the entry signature change is
  invisible at the Yashy level.
- Bundled stdlib discovery works with `YASHY_PATH` unset.
- The first-CLI guide's argument section works with `yashy run --`.
- The full suite is green on Windows and Linux.
- The spec and docs record the R1 `Result` exception.

---

## 7. Risks and rollback

| Risk | Mitigation |
| --- | --- |
| Entry signature breaks every golden IR test | Update the assertions once; add a `-> int` and recursive-`main` test; run the full e2e suite. |
| Windows `char**` is not UTF-8 | Document the code-page limitation in R1; `getcwd()` still returns UTF-8 via the wide API; revisit with Unicode in 1.1. |
| `args()` ownership is wrong | Use the same slice-of-strings path as `split`/`lines`; leak tests with `YASHY_LEAK_REPORT=1`. |
| `getcwd` buffer growth loops | Cap the retry (e.g., 1 MiB) and return the empty string on failure. |
| `sleep` is interrupted | Retry on `EINTR`; test a short sleep. |
| Stale runtime cache | The source hash changes the key; verify with a clean cache and record it. |
| Runtime tests are blocked by Application Control locally | Use the padded launch helper; CI is the cross-platform authority. |

**Rollback:** the runtime functions and builtins are additive. The entry
signature is the only invasive change; if it cannot be stabilized, revert to
`i32 @main()` and use a lazy `GetCommandLineW`/`/proc/self/cmdline` reader for
`args()` as a fallback. Record the decision before falling back.

---

## 8. Files touched

- `crates/yashy-runtime/runtime.c`
- `crates/yashy-runtime/include/yashy_runtime.h`
- `crates/yashy-runtime/src/lib.rs`
- `crates/yashy-runtime/tests/args.rs` (new)
- `crates/yashy-runtime/tests/time.rs` (new)
- `crates/yashy-codegen/src/fn_gen.rs`
- `crates/yashy-codegen/src/module.rs`
- `crates/yashy-codegen/src/expr_gen.rs`
- `crates/yashy-codegen/tests/codegen.rs`
- `crates/yashy-codegen/tests/externs.rs`
- `crates/yashy-sema/src/check_expr.rs`
- `crates/yashy-sema/src/check_expr/` tests (where the builtin tests live)
- `crates/yashy-stdlib/yashy/args.ya`
- `crates/yashy-stdlib/yashy/time.ya`
- `crates/yashy-stdlib/yashy/os.ya`
- `crates/yashy-stdlib/yashy/fs.ya` (cross-reference, optional)
- `crates/yashy-driver/tests/programs/args.ya` (new)
- `crates/yashy-driver/tests/programs/cwd.ya` (new)
- `crates/yashy-driver/tests/programs/time.ya` (new)
- `crates/yashy-driver/tests/e2e.rs`
- `crates/yashy-driver/tests/leaks.rs`
- `crates/yashy-driver/tests/modules.rs` (no-`YASHY_PATH` case)
- `crates/yashy-driver/tests/tutorial.rs`
- `docs/tutorial/12-first-cli.md`
- `docs/superpowers/specs/2026-10-25-yashy-super-language-v2-design.md`
- `README.md`

---

## 9. Definition of done

- The acceptance commands in §5 pass on Windows and Linux.
- The runtime cache is cold-started successfully.
- Bundled stdlib discovery is tested with `YASHY_PATH` unset.
- The first-CLI guide's argument section is tested.
- The spec records the R1 `Result` exception.
- The master plan's gap table marks slice 2 complete.
- Commit and push; record the commit hash in this plan.
