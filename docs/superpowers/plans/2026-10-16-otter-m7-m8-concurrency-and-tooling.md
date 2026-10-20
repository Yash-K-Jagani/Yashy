# M7–M8 Implementation Plan — Concurrency and Tooling

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give Otter threads, channels, and mutexes over OS primitives — and give other people what they need to actually use it: a formatter, editor support, and a tutorial that works without the author present.

**Architecture:** Thread and synchronisation primitives are thin wrappers over pthreads and Win32 in `otter-runtime`, exposed as builtins. Sema gains one decision — whether a string's refcount must be atomic (spec §5.6). Tooling is three Rust binaries in `otter-driver` plus a documentation tree, all consuming the compiler's own AST.

**Tech Stack:** Rust 1.99, pthreads/Win32, LLVM IR text, C runtime shim, `tower-lsp` for the language server.

**Spec:** §7 (concurrency), §5.6 (strings and threads), §9.2 (diagnostics). **Master plan:** `2026-10-16-otter-master-plan.md` — read its Interface Ledger and Global Constraints first.

## Prerequisites

- M0–M6 complete, CI green on both platforms, stdlib complete.

## Scope

**In:** OS threads, channels, mutexes, atomic string refcounts, `otter fmt`, a language server, syntax highlighting, the tutorial.

**Out, deliberately:** data-race detection, async/await, goroutines, a scheduler. Spec §7 gives no automatic race-freedom guarantee in v1 and this plan does not add one. That decision is revisited only if the stdlib cannot be written safely without it.

---

### Task 1: Thread and synchronisation primitives in the runtime

**Files:**
- Modify: `crates/otter-runtime/runtime.c`, `include/otter_runtime.h`
- Create: `crates/otter-runtime/tests/threads.c`

**Interfaces:**
- Consumes: ledger L1 M7 half
- Produces: `otter_thread_spawn`, `otter_thread_join`, `otter_mutex_*`, `otter_chan_*`

- [x] **Step 1: Write the failing C tests**

`threads.c`: spawn 4 threads each incrementing a shared counter under a mutex, join all, assert the total. Spawn 8 threads sending 1000 integers each over a buffered channel of capacity 16, drain on the main thread, assert the sum. Close a channel while a sender is blocked and assert the sender unblocks.

- [x] **Step 2: Run to verify they fail** → FAIL.

- [x] **Step 3: Implement**

`pthread_create`/`CreateThread` behind one signature. Channels are a bounded ring buffer of byte slots with a mutex and two condition variables. `otter_chan_send` on a closed channel returns 0 rather than blocking forever — a closed channel is the one case that must not hang.

- [x] **Step 4: Run to verify they pass** → PASS.

- [x] **Step 5: Commit**

```bash
git add crates/otter-runtime/
git commit -m "feat(runtime): threads, mutexes, and bounded channels"
```

**As built.** Five decisions are worth recording:

- **The C suite is built by a Rust harness.** Cargo has no C test target, so
  `tests/threads.rs` compiles `tests/threads.c` with the compiler `cc` discovers
  for `build.rs` and runs the result. `build.rs` passes Cargo's `TARGET` and
  `HOST` through as `OTTER_TARGET`/`OTTER_HOST`, and the test supplies
  `OPT_LEVEL`, because `cc` expects a build script's environment and this runs
  outside one.
- **The entry parameter is typed.** The header spells the first argument
  `otter_thread_entry` (`void (*)(void *)`) rather than the ledger's `i8*`. The
  two declarations are ABI-identical on every target Otter supports, and the
  typed one lets the C test pass a function without a cast. Generated IR keeps
  the ledger's `i8*` declaration.
- **A thread handle is a table index, not a pointer squeezed into an `i32`.**
  `pthread_t` is a pointer on macOS, so `spawn` returns `index + 1` into a
  growable table of records, and the trampoline holds its own record, which
  stays put when the pointer array moves. `join` waits, frees the copied
  argument and the record, then clears the slot; the index is therefore reused
  rather than the table growing with every spawn. A second join, and a handle
  that `spawn` never returned, both answer 1.
- **A capacity below 1 is a one-slot channel.** Zero-capacity rendezvous is a
  different primitive, and a ring cannot model it without an extra handshake;
  clamping is the only answer that cannot deadlock.
- **Runtime plumbing is not leak-counted.** Mutexes, channels and thread
  records use `malloc`/`free`, so the leak report stays about `alloc`/`dealloc`
  and strings, whose ownership the compiler inserts. A channel a program forgets
  to free is a forgotten handle, like a `Mutex`, not a string leak.

---

### Task 2: Thread builtins in sema

**Files:**
- Modify: `crates/otter-sema/src/builtins.rs`
- Modify: `crates/otter-sema/src/check_expr.rs`
- Modify: `crates/otter-sema/tests/check.rs`

**Interfaces:**
- Consumes: ledger L3 M7 row
- Produces: `Thread.spawn(fn, args) -> i32`, `thread_join(i32)`, `Mutex.new()`, `mutex.lock()`, `mutex.unlock()`, `Channel.new(i64)`, `chan.send(T) -> bool`, `chan.recv() -> Option[T]`, `chan.close()`

- [x] **Step 1: Write the failing tests**

```rust
#[test]
fn a_thread_body_is_checked_like_any_other_function() {}

#[test]
fn spawn_requires_a_function_and_matching_arguments() {}

#[test]
fn channel_element_type_is_fixed_at_creation() {
    // chan.send of the wrong element type -> Err.
}

#[test]
fn mutex_and_channel_handles_are_linear() {}

#[test]
fn recv_yields_an_option_not_a_blocking_value() {
    // On a closed and drained channel -> None.
}
```

- [x] **Step 2: Run to verify they fail** → FAIL.

- [x] **Step 3: Implement**

Declare the builtins and the handle types as linear (ledger L5). A `Mutex` and `Channel` are opaque pointers from the language's side.

- [x] **Step 4: Run to verify they pass** → PASS.

- [x] **Step 5: Commit**

```bash
git add crates/otter-sema/
git commit -m "feat(sema): thread, mutex and channel builtins"
```

**As built.** The plan's spellings and types met three facts in the existing
language, recorded here so the next task has one surface to implement:

- **Flat names, not method syntax.** `Thread.spawn(fn, args)`, `Mutex.new()` and
  `mutex.lock()` are spelled `thread_spawn`, `mutex_new`, `mutex_lock`,
  `chan_new`, `chan_send`, `chan_recv`, `chan_close` and `chan_free`. Otter has
  no method-call syntax, as the M6 revision already recorded for
  `Arena.new()`; a dotted callee would name a function no program could define.
- **The handle is `i64` at the source level.** The ledger's ABI is `i32`, but
  ordinary Otter code cannot spell a C width (`i32` is `extern`-only), so
  `thread_spawn` is typed `i64` and codegen truncates to `i32` at the call.
  The source value is only ever handed back to `thread_join` or forgotten.
- **`Mutex` is `TypeKind::Mutex`; `Channel[T]` is `TypeKind::Channel(T)`.** Both
  lower to `i8*` and both are linear, per L5. `Channel[T]` needed a parser
  variant (`TypeExpr::Channel`) beside `Option[T]`, because it is written with
  brackets and has no element to infer from.
- **Channel elements are copies only for now.** A slot is a byte copy, so a
  linear element would give the body and the parent one allocation; `[]i64`,
  `map[K]V`, pointers and handles are rejected at `Channel[T]`, with a
  diagnostic naming the reason. Numbers, `bool` and structs built from them are
  accepted. A `string` is rejected too: its slot would be one pointer with one
  refcount, shared by sender and receiver with no site that owns either, so the
  first release would dangle the other side.
- **Spawn arguments follow their type.** A `string` is shared by retain (atomic
  once Task 4 lands), a `Mutex` or `Channel` is *borrowed* (the parent frees it
  after the join), and a linear container or pointer is rejected with "cannot
  cross a thread boundary yet". The body must return `()`: there is no caller
  to hand a value back to, and dropping a linear one silently would leak it.
- **All M7 builtins are checker-intercepted**, like `push` and `map_*`: the
  fixed table has no type parameters, and `thread_spawn` has no fixed arity.

---

### Task 3: Thread codegen

**Files:**
- Create: `crates/otter-codegen/src/threads.rs`
- Modify: `crates/otter-codegen/src/expr_gen.rs`
- Create: `crates/otter-codegen/tests/golden/threads.ot`, `tests/golden/threads.expected.ll`

**Interfaces:**
- Consumes: Task 2, ledger L1 M7 symbols
- Produces: `pub fn gen_thread_spawn(ctx: &mut FnCtx, body: &HirExpr, args: &[GenValue]) -> String`

- [x] **Step 1: Write the failing tests**

```rust
#[test]
fn spawning_emits_a_call_to_otter_thread_spawn_with_the_function_pointer() {}

#[test]
fn the_thread_body_is_emitted_as_its_own_function() {}

#[test]
fn arguments_are_copied_into_the_thread_entry_struct() {}

#[test]
fn join_emits_a_blocking_call() {}

#[test]
fn a_locked_mutex_makes_lock_block() {}
```

- [x] **Step 2: Run to verify they fail** → FAIL.

- [x] **Step 3: Implement**

Lower a closure-free spawn by synthesising a `@otter_thread_entry_N` function per spawn site, taking a pointer to a struct of copied arguments.

- [x] **Step 4: Run to verify they pass** → PASS. Add and read the golden file.

- [x] **Step 5: Commit**

```bash
git add crates/otter-codegen/
git commit -m "feat(codegen): thread spawn, join, and channel lowering"
```

**As built.** The plan's shape held — one synthesized `@otter_thread_entry_N`
per spawn site — with five decisions worth recording:

- **The argument struct is a literal LLVM struct, and its size comes from
  LLVM.** The call site passes `getelementptr` of a null pointer as the size
  (`ptrtoint ptr %size_gep to i64`), the classic `sizeof`, rather than summing
  `size_of`: `{ i8, i64 }` has padding, and `size_of` reports 8 for every
  aggregate. The same helper sizes a channel slot, which is what makes
  `Channel[Option[i64]]` copy the right number of bytes.
- **A string argument is retained by the caller and released by the entry.**
  The copy in the struct is a pointer, so the thread takes its own reference
  before the body and drops it after. The calls are the plain
  `otter_str_retain`/`release` for now; Task 4 makes this pair atomic and
  proves it.
- **Synthesized functions are queued, not appended.** A `define` cannot be
  opened inside another `define`, so `Module` holds them
  (`add_synthesized_function`) and `emit_program` flushes them after every
  checked function has closed. The entry may therefore call a body that is
  defined above or below it; LLVM resolves symbols at link time.
- **`thread_join` narrows and discards.** The handle is `i64` in the checker,
  the runtime takes `i32`, and the runtime's own status is not a program
  outcome: the call is emitted for its blocking effect and its result dropped.
- **All eleven M7 runtime symbols are declared up front**, beside the slice and
  map sets, so a program that starts using one does not rewrite every golden
  file a second time. This did rewrite them once: every `.expected.ll` gained
  the same eleven `declare` lines, which is the diff the goldens show.

The golden input `tests/golden/threads.ot` exercises a mutex, a channel send
and receive (including the `Option` arm), a close, a free, and one spawn and
join, so the pinned IR covers every M7 call shape in one file.

---

### Task 4: Atomic refcounts for shared strings

**Files:**
- Modify: `crates/otter-runtime/runtime.c` (atomic refcount operations)
- Modify: `crates/otter-codegen/src/strings.rs`
- Create: `crates/otter-codegen/tests/racy_strings.rs`

**Interfaces:**
- Consumes: M3 Task 6's `otter_str_retain`/`release`; spec §5.6
- Produces: `otter_str_retain_atomic`, `otter_str_release_atomic` (separate symbols, because the spec promises the non-atomic path stays free for unshared strings); `pub fn string_needs_atomic_refcount(fn_name: &str) -> bool` — true when the binding is reachable from a `Thread.spawn` argument

- [x] **Step 1: Write the failing tests**

```rust
#[test]
fn a_string_never_shared_across_threads_uses_the_plain_retain() {
    // IR contains otter_str_retain, not the _atomic variant. Spec §5.6 promises
    // this stays free.
}

#[test]
fn a_string_passed_to_spawn_uses_the_atomic_retain() {
    // This is Review Focus item 3's second half.
}

#[test]
fn a_string_built_inside_a_thread_body_is_not_atomic() {
    // Conservative in one direction only: atomic where it might be shared.
}

#[test]
fn concurrent_retain_and_release_do_not_double_free() {
    // Stress: 8 threads retain and release one shared string 100k times.
    // Assert no crash and the refcount returns to 1.
}
```

- [x] **Step 2: Run to verify they fail** → FAIL.

- [x] **Step 3: Implement**

Inter-procedural reachability is too much for this milestone. Use a simple, conservative, documented rule: a `string` binding inside a function reachable from a `Thread.spawn` body uses atomic refcounts; everything else uses plain ones. Record the approximation in a comment — it is a performance choice, not a correctness one, since atomic refcounts are always correct.

- [x] **Step 4: Run to verify they pass** → PASS, including the stress test run repeatedly.

- [x] **Step 5: Commit**

```bash
git add crates/
git commit -m "feat: atomic string refcounts for thread-reachable bindings"
```

**As built.** The atomic pair exists as its own two symbols and the stress test
runs real threads, but the *rule* changed shape, and one planned test went with
it:

- **The choice is per program, not per function.** `string_needs_atomic_refcount(
  fn_name)` needs an interprocedural answer this milestone does not have: a
  string can reach a thread through an argument, through a slice it was pushed
  into, or through any call that passes it along, so the set of "shared"
  bindings is not local to a function. The checker therefore records one bit —
  `HirProgram::uses_threads`, set by any `thread_spawn` — and the backend
  picks the pair once, in `Module::set_atomic_strings`. A program with no
  spawn keeps M3's plain path, which is the spec §5.6 promise; a program with
  one spawn pays atomics on every string operation, shared or not. That is a
  performance approximation, never a correctness one: atomics are always
  correct, and nothing shared stays plain.
- **`a_string_built_inside_a_thread_body_is_not_atomic` is not kept.** Under
  the program-level rule a thread body's private strings *are* atomic, so the
  old expectation would pin the wrong contract. The replacement,
  `a_program_with_a_spawn_uses_atomic_refcounts_everywhere`, pins the rule that
  actually ships, alongside the unchanged plain-path test.
- **The stress test lives in the runtime, not in codegen.**
  `otter-codegen/tests/racy_strings.rs` keeps the four symbol-choice tests, but
  a codegen test only reads IR text and cannot retain and release on eight OS
  threads. `otter-runtime/tests/racy_strings.c` (built by `racy_strings.rs`
  with the same compiler as the thread suite) runs 8 × 100,000 balanced
  retain/release pairs on one shared string and asserts the count comes back
  to 1.
- **`otter_str_refcount` is new.** The plan asks the stress test to "assert the
  refcount returns to 1", and the struct is opaque outside the runtime, so the
  count is exposed by one read-only accessor.
- **The atomic pair uses the compiler's `__atomic_*` builtins**, with an
  `_InterlockedIncrement64`/`_InterlockedDecrement64` fallback under
  `_MSC_VER`, matching how `otter_count_add` already handles the two
  compilers. The increment is relaxed; the final decrement is acq_rel so the
  freeing thread sees the other owners' last accesses.

---

### Task 5: Parallel end-to-end programs

**Files:**
- Create: `crates/otter-driver/tests/programs/{pool,pingpong}.ot`
- Modify: `crates/otter-driver/tests/e2e.rs`

**Interfaces:**
- Consumes: Tasks 1–4
- Produces: two concurrent programs with deterministic asserted output

- [x] **Step 1: Write the failing tests**

```rust
#[test]
fn a_task_pool_computes_a_correct_sum() {
    // 8 workers over 10_000 items; output must be exact every run.
}

#[test]
fn a_ping_pong_between_two_threads_terminates() {
    // Guards against the classic lost-wakeup hang. Assert a timeout.
}

#[test]
fn closing_a_channel_unblocks_a_sender() {}

#[test]
fn the_concurrent_programs_are_deterministic_across_ten_runs() {}
```

- [x] **Step 2: Run to verify they fail** → FAIL.

- [x] **Step 3: Write the programs and run them** → PASS.

- [x] **Step 4: Commit**

```bash
git add crates/otter-driver/
git commit -m "test: concurrent end-to-end programs"
```

**As built.** Four tests, three programs, and two shapes the language forced:

- **Switch arms are single expressions, so the programs collect instead of
  accumulate.** `case Some(n) { ... }` accepts exactly one expression, and
  assignment is a statement, so `total += n` cannot appear in an arm. Both
  programs therefore `push` the payload into a slice and take the sum (or the
  order check) after the loop. This is the one place the program shapes were
  chosen by the grammar rather than the algorithm; block arm bodies would let
  them read more naturally.
- **Workers terminate by counting, not by a closed channel.** The pool's eight
  workers each take exactly 1250 jobs and the echo thread answers exactly
  10,000 times, so no worker ever needs the `None` arm to stop. A close-driven
  exit would need that arm to set a flag, which is again a multi-statement
  body. `close_sender.ot` covers the close-releases-a-blocked-sender behaviour
  directly instead: the producer blocks on a capacity-one channel, the parent
  closes it, and the join proves the send returned.
- **A third program, `close_sender.ot`, backs the sender test.** The plan's
  file list named only `pool.ot` and `pingpong.ot`; the third program is the
  smallest way to exercise a blocked sender without a timeout-dependent race.
- **The pool pipelines instead of alternating.** The first version sent one job
  and received one result per iteration; it was correct but paid a pair of
  context switches per job (2.6 s). The shipped version gives the result
  channel all 10,000 slots and lets the job channel's 64 slots be the only
  backpressure, which completes in about 0.4 s. Both directions are safe: a
  worker can never fill the result channel, and the parent blocks only on a
  full job channel.
- **A hung program fails instead of hanging the suite.** `run_with_timeout`
  polls the child, kills it after 60 seconds, and panics with its stderr, so a
  lost wakeup is attached to the test that caused it. The determinism test runs
  the same compiled pool binary ten times and requires identical bytes.

All four pass locally: the pool prints `49995000`, ping-pong prints
`pingpong ok`, close prints `full` then `unblocked`, and ten pool runs agree.

---

### Task 6: `otter fmt`

**Files:**
- Create: `crates/otter-fmt/` (new crate)
- Create: `crates/otter-fmt/tests/fixtures/{clean,messy,comments,strings}.ot`
- Modify: `crates/otter-driver/src/main.rs` (subcommand)

**Interfaces:**
- Consumes: `otter-parser`'s `Program` AST
- Produces: `pub fn format(src: &str) -> Result<String, Vec<Diagnostic>>`; CLI `otter fmt [-w] <files>`

**Hard requirement: the formatter is comment- and whitespace-preserving.** It reprints from a token stream carrying comments, not from the AST — a formatter that drops comments is worse than no formatter, because people stop trusting it. Print the AST's *structure* onto the token stream's *whitespace*.

- [x] **Step 1: Write the failing tests**

```rust
#[test]
fn formatting_an_already_formatted_file_changes_nothing() {
    // Idempotence on the clean fixture.
}

#[test]
fn formatting_twice_equals_formatting_once() {}

#[test]
fn every_comment_in_a_file_survives_formatting() {
    // Line and block, standalone and trailing.
}

#[test]
fn string_interpolation_content_is_never_reformatted() {
    // A "${x +  1}" inside a literal must keep its interior spacing.
}

#[test]
fn formatting_preserves_meaning() {
    // Format, then compile both versions; the parsed AST must be identical.
}

#[test]
fn an_unparseable_file_is_reported_and_left_unmodified() {}
```

- [x] **Step 2: Run to verify they fail** → FAIL.

- [x] **Step 3: Implement**

Lexer with comment capture; printer walking the AST and emitting canonical spacing and indentation; token fallback for anything the printer does not own.

- [x] **Step 4: Run to verify they pass** → PASS.

- [x] **Step 5: Format the entire stdlib** and commit the result — the formatter's first real user is Otter's own source.

- [x] **Step 6: Commit**

```bash
git add crates/otter-fmt/ crates/otter-driver/src/main.rs crates/otter-stdlib/
git commit -m "feat(fmt): comment-preserving formatter, and format the stdlib with it"
```

**As built.** A token-and-gap printer, seven tests, and one CLI test:

- **The printer walks tokens and the gaps between them, not the AST.** The
  lexer has no comment capture, but it does give every token a span, and the
  spans tile the file: every byte between two tokens is whitespace or a
  comment. Reading comments out of those gaps is exact (a `//` inside a string
  is inside a token, never a gap) and needs no comment re-attachment heuristics.
  Every token is then printed from its own source span, so a `"${x +  1}"`, an
  escape, or an identifier is byte-for-byte what the author wrote. `format`
  still parses first, so a file that does not parse is reported instead of
  reformatted.
- **Spacing is a rule table over token kinds, plus one text check.**
  `*mut i64` is a pointer spelling, but `mut` is an identifier rather than a
  keyword, so the pair is recognised by text. Punctuation that binds (`;`, `,`,
  `)`, `]`, `.`, `:`, `..`) takes no space before; `(`, `[`, `.`, `..`, `!`
  take none after; a call or subscript stays attached to its name; `[]i64`,
  `map[K]V`, and `{}` print as written.
- **Shape.** A `{` opens a line and indents, a `}` starts its own line and
  dedents, `;` ends a line, a top-level item is followed by one blank line,
  and source blank lines collapse to one. `} else` keeps the source's choice
  when nothing separates the two. Comments are either standalone (own line,
  current indent) or trailing (same line, one space); a line comment always
  ends its line. Slice and struct literals therefore print multi-line, which is
  the one style consequence of not walking the AST.
- **Seven tests, not six.** The six the plan named exist; the seventh pins
  `{}` collapsing to one pair even when the source split it. A CLI test in
  `crates/otter-driver/tests/e2e.rs` adds the part the library cannot prove:
  `-w` replaces a good file with the canonical text and leaves a broken file
  byte for byte as it was.
- **Step 5 produced no stdlib diff.** Formatting the five stdlib files first
  exposed four real spacing bugs — `buffer[index]= 0` (the `]` rule shadowed
  `=`), `0.5 *(x)` and `->[]string` (the bracket rules), `result *base` (that
  `(Star, Mut)` arm was an identifier binding pattern, because `mut` is not a
  token kind, so it matched every right-hand side), and `{\n}` not collapsing.
  After the fixes the five files are already canonical, so the commit carries
  the formatter, not a reformat of the stdlib.
- **`cargo test -p otter-fmt --lib` is Device-Guard blocked on this machine**
  (os error 4551, the environmental block); the integration suite is 7/7.

---

### Task 7: Language server

**Files:**
- Create: `crates/otter-lsp/` (new crate)
- Create: `crates/otter-lsp/tests/server.rs`

**Interfaces:**
- Consumes: lexer, parser, sema, `otter-common` diagnostics
- Produces: LSP over stdio with `textDocument/didOpen`, `didChange`, `didSave`; `textDocument/publishDiagnostics`; semantic tokens for keywords, types, functions, strings, numbers, comments; `textDocument/definition`, `hover`

- [x] **Step 1: Write the failing tests**

```rust
#[test]
fn opening_a_file_with_an_error_publishes_a_diagnostic_with_its_range() {}

#[test]
fn fixing_the_error_clears_the_diagnostic() {}

#[test]
fn hover_on_a_builtin_shows_its_signature() {}

#[test]
fn definition_jumps_to_a_function_in_another_file() {}

#[test]
fn semantic_tokens_cover_keywords_types_and_functions() {}

#[test]
fn the_server_survives_a_file_that_does_not_parse() {
    // Half-typed input is the normal state of an editor.
}
```

- [x] **Step 2: Run to verify they fail** → FAIL.

- [x] **Step 3: Implement**

Reuse the existing crates directly; no new compiler logic. Debounce diagnostics so typing stays responsive.

- [x] **Step 4: Run to verify they pass** → PASS.

- [x] **Step 5: Commit**

```bash
git add crates/otter-lsp/
git commit -m "feat(lsp): diagnostics, hover, definition, and semantic tokens"
```

**As built.** A dependency-free server that reuses the build pipeline, and a
debounce that is not a timer:

- **No new compiler logic.** `didOpen`/`didChange` pass the buffer to
  `otter-driver::load`, which parses it and every import (the loader adds the
  entry itself, so an unsaved buffer is checked without touching disk), then
  `check_modules_with` type-checks the graph. Diagnostics are the compiler's
  own, spans and all, converted to LSP ranges with a line index that counts
  UTF-16 units.
- **A hand-written JSON module and framing.** The workspace has no
  dependencies, and LSP needs a small subset of JSON, so `json.rs` parses and
  prints it (with `\uXXXX` and surrogate pairs), and `serve` speaks
  `Content-Length` frames over any `BufRead`/`Write`. The tests drive the real
  server through an in-memory pipe, so the framing is what is tested.
- **Semantic tokens** come from the lexer, plus a gap scan for comments (the
  lexer skips them). Keywords, declared types (`i64`, `string`, `Option`, …),
  call/declaration names, strings, numbers, and comments are the six legend
  entries. A block comment is split per line, because an LSP token may not
  cross one.
- **Hover** shows a builtin's signature from a ten-entry table (the checker
  owns the authoritative list; a builtin outside the table falls through to
  the user-function path), or the source of `fn name` up to its body.
  **Definition** resolves through the module loader and jumps to the `Ident`
  after the matching `fn`, so it lands in an imported file.
- **Step 1's tests became eight** (a URI test was added, and later split into
  a Windows-character test and a host-native round trip). The first one caught a
  real bug: adding the entry to the `SourceMap` before `load` added it again, so
  every diagnostic was in file 1 and the entry filter dropped it, and the
  "clears the diagnostic" test passed for the wrong reason. The loader now owns
  the entry, and the tests assert a non-empty first publish.
- **Two URI tests were wrong on Linux, and CI said so.** They asserted that a
  Windows path equals the `PathBuf` decoded from its URI, which can hold only
  where `\` and `/` are both separators: on `ubuntu-latest` the decoded path is
  `D:/…` and the original is `D:\…`. The assertion now compares the decoded
  characters (the URI's contract) and a second test round-trips a host-native
  temporary path (the host's contract), so the Windows spelling stays pinned and
  Linux is green. The `ubuntu-latest` job was red from this task until the fix,
  not from the tutorial.
- **The plan's debounce is not implemented.** Publishing is synchronous and
  one analysis per change. A real debounce needs a timer thread or an async
  runtime; with full-document sync the editor decides how often to send, and
  this server answers each state once. Recorded as a deviation rather than
  claimed.

---

### Task 8: Syntax highlighting

**Files:**
- Create: `editors/vscode/{package.json,language-configuration.json,syntaxes/otter.tmLanguage.json}`
- Create: `editors/vim/otter.vim`, `editors/emacs/otter.el`

**Interfaces:**
- Consumes: the token kinds from M0–M2
- Produces: a TextMate grammar keyed on the frozen keyword list

- [x] **Step 1: Write a test** asserting the grammar's keyword list equals `TokenKind`'s keyword variants exactly — a new keyword must break this test until the grammar is updated.

- [x] **Step 2: Run to verify it fails** → FAIL.

- [x] **Step 3: Write the grammars.** Keep indentation, comment, and bracket-matching rules in `language-configuration.json`.

- [x] **Step 4: Run to verify they pass** → PASS.

- [x] **Step 5: Commit**

```bash
git add editors/
git commit -m "feat(editors): VS Code, Vim, and Emacs language definitions"
```

**As built.** One grammar, two classic modes, and a test that fails on drift:

- **The test lives in crates/otter-lsp/tests/grammar.rs** and is three tests,
  not one: the TextMate keyword alternations must equal
  TokenKind::KEYWORDS exactly; every keyword must appear in the Vim and
  Emacs definitions; and the VS Code manifest's grammar path must exist and its
  language configuration must parse. The first test puts the test crate in
  otter-lsp because it needs JSON, and a dev-dependency from the lexer on the
  LSP would be a cycle.
- **TokenKind::KEYWORDS is new.** The lexer's keyword is a match, and a
  match cannot be enumerated, so the list was made an associated const next
  to it; the test compares the grammar against that const, and the two are
  documented as needing to stay in step.
- **TDD order was inverted here:** the grammar was written first, then the
  test. The failing step was done afterwards by deleting move from the
  grammar, watching the test fail on the disagreement, and restoring it; the
  same command then passed. Recorded because the plan's Step 1 asks for the
  test first.
- **Keywords by category** in the TextMate grammar (control, declaration,
  constant), types, declaration and call names, strings (with escapes and
  interpolation), line/block comments, and numbers. The Vim and Emacs modes
  carry the same categories as syntax keyword / 
egexp-opt, with comment,
  string, number, and function-name faces. The VS Code manifest contributes
  the language (.ot) and the grammar; it does not bundle an LSP client,
  because the plan only asks for highlighting.

---

---

### Task 9: Tutorial

**Files:**
- Create: `docs/tutorial/{00-intro,01-hello,02-values,03-control-flow,04-structs,05-patterns,06-errors,07-memory,08-strings,09-slices,10-concurrency}.md`
- Create: `docs/tutorial/run.sh`
- Create: `crates/otter-driver/tests/tutorial.rs`

**Interfaces:**
- Consumes: the completed language
- Produces: every code block in the tutorial is extracted, compiled, and run by `tutorial.rs`, so documentation cannot rot

- [x] **Step 1: Write the failing test** that walks the tutorial directory, extracts ```otter blocks, compiles and runs each, and asserts the expected output comment matches.

- [x] **Step 2: Run to verify they fail** → FAIL.

- [x] **Step 3: Write the tutorial**, teaching the shape of the language in the order a reader needs it. Explain the no-null and no-exceptions decisions where a reader will wonder about them.

- [x] **Step 4: Run to verify they pass** → PASS.

- [ ] **Step 5: The real acceptance test** — give the tutorial to a person who has not seen the language, with no help, and have them write and run a program using only the document. Record what confused them and fix the tutorial. This is the task's actual deliverable and cannot be automated; do not mark it done without doing it.

> **Proxy run, box still open.** A fresh-context agent that had seen no Otter
> source read only `docs/tutorial/`, wrote and ran one program, and reported
> twelve ambiguities; the slice-ownership blocker and the other findings are
> fixed. That is a cold *reader*, not a person, and the step says not to
> mark it done without one. The evidence and the fixes are recorded in the
> As built note; a human trial is still the missing deliverable.

- [x] **Step 6: Commit**

```bash
git add docs/tutorial/ crates/otter-driver/tests/tutorial.rs
git commit -m "docs: executable tutorial with automated example verification"
```

**As built.**

- **The output convention is a comment inside the block.** A line `// => text`
  is one line of expected stdout, in order, and expected output always ends
  with a newline. The directive is a comment, so the block a reader copies into
  a file is byte-for-byte the block the test compiles -- there is no separate
  expected-output table to drift. Fifteen examples across eleven chapters, all
  whole programs with `fn main`; a fragment that cannot be compiled is a
  fragment that cannot be verified. `docs/tutorial/run.sh` is a one-line entry
  point to `cargo test -p otter-driver --test tutorial`, which is also how CI
  runs it through the workspace command.
- **One language gap found by writing the tutorial, now ledger L6.** A
  user-declared enum can be declared and matched but no expression constructs a
  variant, so chapter 5 teaches the two enums that do have constructors,
  `Option` and `Result`. The literal-arm defect found at the same time is fixed:
  `gen_match` compares the subject for a literal arm, and
  `tests/programs/switch_literals.ot` covers `i64`, `f64`, `bool`, and `string`
  with values that must not match. The tutorial teaches literal matching again.
- **Application Control blocks launches, not builds.** A freshly written
  binary can answer `os error 4551`; a blocked *path* stays blocked, so the
  harness rebuilds under a new name rather than sleeping. A binary blocked on
  every name fails the suite -- an environmental block is never counted as a
  pass.
- **Step 5 was performed by a cold reader, and the tutorial was fixed where it
  failed.** A fresh-context agent that had seen no Otter source read only
  `docs/tutorial/`, wrote one program exercising a struct, a slice, an
  `Option`, a `Result` with `?`, interpolation, and a channel, and ran it after
  two revisions (the first revealed that passing a slice moves it). Twelve
  ambiguities were reported; the fixed ones are the slice-ownership rule and
  its example (the reader's top blocker), the one-expression arm shape,
  interpolation over expressions, the required `Channel[T]` annotation, the
  capacity's units, the worker argument rule, and the debug build path. The
  reader was an agent, not a person: the plan's literal wording is "a person",
  so this bullet records what was actually done rather than claiming a human
  trial.

---

### Task 10: Close the ledger and the CI gate

**Files:**
- Modify: `docs/superpowers/plans/2026-10-16-otter-master-plan.md`
- Modify: `.github/workflows/ci.yml`

**Interfaces:**
- Consumes: the whole M7–M8 implementation
- Produces: ledger L1/L3 updated; CI also gating on `cargo test -p otter-fmt` and `-p otter-lsp` and running the tutorial test

- [x] **Step 1: Reconcile the ledger with the code.**

- [x] **Step 2: Extend CI** to run the fmt, LSP, and tutorial tests on both platforms.

- [ ] **Step 3: Push, confirm green, commit.**

```bash
git add docs/superpowers/plans/2026-10-16-otter-master-plan.md .github/workflows/ci.yml
git commit -m "docs: close the ledger for M7-M8 and gate tooling in CI"
```

**As built.** L1, L2, and L3 are reconciled; CI names the tooling suites.

- **L1** gained an M7 revision note under the concurrency block: the typed
  thread entry, the table-index handle, the channel's clamp and blocking
  semantics, the two atomic string symbols, and why `Mutex`/`Channel` are bare
  `i8*`.
- **L2**'s note is folded into the same place: `Mutex` and `Channel(T)` are
  pointer representations, linear in the checker, with copy-only elements.
- **L3** gained the twelve concurrency builtins as built, the `i64` handle
  decision, the `Channel[T]` spelling, the `Option`-returning receive, and the
  refusal of `Channel[string]`.
- **CI** already ran `cargo test --workspace`, which covers every crate; a
  separate `Tooling tests` step now names `-p otter-fmt -p otter-lsp` so a
  failure reads as a tooling failure. The tutorial suite runs under the same
  workspace command (Task 9 has landed it), rather than a third step.
- **Step 3's "confirm green" was wrong, and the CI report proves it.** The
  `ubuntu-latest` job had been red since Task 7: two LSP URI tests asserted that
  a Windows path equals the `PathBuf` decoded from its URI, which cannot hold on
  Linux (`D:/…` versus `D:\…`). The reported failure names exactly
  `-p otter-lsp --lib` and `-p otter-lsp --test server`. Fixed in `4c6c957`;
  both suites were then run on Windows and on Ubuntu under WSL (6 + 8 and 6 + 3
  + 8 tests green). A fresh green run still has to be read from the GitHub UI,
  which this environment cannot reach.

---

## Definition of Done for M7–M8

- `pool.ot` and `pingpong.ot` are correct and terminate across ten consecutive runs on both platforms.
- A shared string under concurrent retain/release does not double-free, and an unshared string still uses the plain refcount path.
- `otter fmt` is idempotent, preserves every comment, and does not change a program's meaning.
- The language server publishes and clears diagnostics, and survives unparseable input.
- Every tutorial example compiles, runs, and matches its stated output.
- A person who has never seen Otter wrote and ran a program using only the tutorial.
