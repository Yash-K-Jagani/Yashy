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

- [ ] **Step 1: Write the failing C tests**

`threads.c`: spawn 4 threads each incrementing a shared counter under a mutex, join all, assert the total. Spawn 8 threads sending 1000 integers each over a buffered channel of capacity 16, drain on the main thread, assert the sum. Close a channel while a sender is blocked and assert the sender unblocks.

- [ ] **Step 2: Run to verify they fail** → FAIL.

- [ ] **Step 3: Implement**

`pthread_create`/`CreateThread` behind one signature. Channels are a bounded ring buffer of byte slots with a mutex and two condition variables. `otter_chan_send` on a closed channel returns 0 rather than blocking forever — a closed channel is the one case that must not hang.

- [ ] **Step 4: Run to verify they pass** → PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/otter-runtime/
git commit -m "feat(runtime): threads, mutexes, and bounded channels"
```

---

### Task 2: Thread builtins in sema

**Files:**
- Modify: `crates/otter-sema/src/builtins.rs`
- Modify: `crates/otter-sema/src/check_expr.rs`
- Modify: `crates/otter-sema/tests/check.rs`

**Interfaces:**
- Consumes: ledger L3 M7 row
- Produces: `Thread.spawn(fn, args) -> i32`, `thread_join(i32)`, `Mutex.new()`, `mutex.lock()`, `mutex.unlock()`, `Channel.new(i64)`, `chan.send(T) -> bool`, `chan.recv() -> Option[T]`, `chan.close()`

- [ ] **Step 1: Write the failing tests**

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

- [ ] **Step 2: Run to verify they fail** → FAIL.

- [ ] **Step 3: Implement**

Declare the builtins and the handle types as linear (ledger L5). A `Mutex` and `Channel` are opaque pointers from the language's side.

- [ ] **Step 4: Run to verify they pass** → PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/otter-sema/
git commit -m "feat(sema): thread, mutex and channel builtins"
```

---

### Task 3: Thread codegen

**Files:**
- Create: `crates/otter-codegen/src/threads.rs`
- Modify: `crates/otter-codegen/src/expr_gen.rs`
- Create: `crates/otter-codegen/tests/golden/threads.ot`, `tests/golden/threads.expected.ll`

**Interfaces:**
- Consumes: Task 2, ledger L1 M7 symbols
- Produces: `pub fn gen_thread_spawn(ctx: &mut FnCtx, body: &HirExpr, args: &[GenValue]) -> String`

- [ ] **Step 1: Write the failing tests**

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

- [ ] **Step 2: Run to verify they fail** → FAIL.

- [ ] **Step 3: Implement**

Lower a closure-free spawn by synthesising a `@otter_thread_entry_N` function per spawn site, taking a pointer to a struct of copied arguments.

- [ ] **Step 4: Run to verify they pass** → PASS. Add and read the golden file.

- [ ] **Step 5: Commit**

```bash
git add crates/otter-codegen/
git commit -m "feat(codegen): thread spawn, join, and channel lowering"
```

---

### Task 4: Atomic refcounts for shared strings

**Files:**
- Modify: `crates/otter-runtime/runtime.c` (atomic refcount operations)
- Modify: `crates/otter-codegen/src/strings.rs`
- Create: `crates/otter-codegen/tests/racy_strings.rs`

**Interfaces:**
- Consumes: M3 Task 6's `otter_str_retain`/`release`; spec §5.6
- Produces: `otter_str_retain_atomic`, `otter_str_release_atomic` (separate symbols, because the spec promises the non-atomic path stays free for unshared strings); `pub fn string_needs_atomic_refcount(fn_name: &str) -> bool` — true when the binding is reachable from a `Thread.spawn` argument

- [ ] **Step 1: Write the failing tests**

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

- [ ] **Step 2: Run to verify they fail** → FAIL.

- [ ] **Step 3: Implement**

Inter-procedural reachability is too much for this milestone. Use a simple, conservative, documented rule: a `string` binding inside a function reachable from a `Thread.spawn` body uses atomic refcounts; everything else uses plain ones. Record the approximation in a comment — it is a performance choice, not a correctness one, since atomic refcounts are always correct.

- [ ] **Step 4: Run to verify they pass** → PASS, including the stress test run repeatedly.

- [ ] **Step 5: Commit**

```bash
git add crates/
git commit -m "feat: atomic string refcounts for thread-reachable bindings"
```

---

### Task 5: Parallel end-to-end programs

**Files:**
- Create: `crates/otter-driver/tests/programs/{pool,pingpong}.ot`
- Modify: `crates/otter-driver/tests/e2e.rs`

**Interfaces:**
- Consumes: Tasks 1–4
- Produces: two concurrent programs with deterministic asserted output

- [ ] **Step 1: Write the failing tests**

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

- [ ] **Step 2: Run to verify they fail** → FAIL.

- [ ] **Step 3: Write the programs and run them** → PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/otter-driver/
git commit -m "test: concurrent end-to-end programs"
```

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

- [ ] **Step 1: Write the failing tests**

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

- [ ] **Step 2: Run to verify they fail** → FAIL.

- [ ] **Step 3: Implement**

Lexer with comment capture; printer walking the AST and emitting canonical spacing and indentation; token fallback for anything the printer does not own.

- [ ] **Step 4: Run to verify they pass** → PASS.

- [ ] **Step 5: Format the entire stdlib** and commit the result — the formatter's first real user is Otter's own source.

- [ ] **Step 6: Commit**

```bash
git add crates/otter-fmt/ crates/otter-driver/src/main.rs crates/otter-stdlib/
git commit -m "feat(fmt): comment-preserving formatter, and format the stdlib with it"
```

---

### Task 7: Language server

**Files:**
- Create: `crates/otter-lsp/` (new crate)
- Create: `crates/otter-lsp/tests/server.rs`

**Interfaces:**
- Consumes: lexer, parser, sema, `otter-common` diagnostics
- Produces: LSP over stdio with `textDocument/didOpen`, `didChange`, `didSave`; `textDocument/publishDiagnostics`; semantic tokens for keywords, types, functions, strings, numbers, comments; `textDocument/definition`, `hover`

- [ ] **Step 1: Write the failing tests**

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

- [ ] **Step 2: Run to verify they fail** → FAIL.

- [ ] **Step 3: Implement**

Reuse the existing crates directly; no new compiler logic. Debounce diagnostics so typing stays responsive.

- [ ] **Step 4: Run to verify they pass** → PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/otter-lsp/
git commit -m "feat(lsp): diagnostics, hover, definition, and semantic tokens"
```

---

### Task 8: Syntax highlighting

**Files:**
- Create: `editors/vscode/{package.json,language-configuration.json,syntaxes/otter.tmLanguage.json}`
- Create: `editors/vim/otter.vim`, `editors/emacs/otter.el`

**Interfaces:**
- Consumes: the token kinds from M0–M2
- Produces: a TextMate grammar keyed on the frozen keyword list

- [ ] **Step 1: Write a test** asserting the grammar's keyword list equals `TokenKind`'s keyword variants exactly — a new keyword must break this test until the grammar is updated.

- [ ] **Step 2: Run to verify it fails** → FAIL.

- [ ] **Step 3: Write the grammars.** Keep indentation, comment, and bracket-matching rules in `language-configuration.json`.

- [ ] **Step 4: Run to verify they pass** → PASS.

- [ ] **Step 5: Commit**

```bash
git add editors/
git commit -m "feat(editors): VS Code, Vim, and Emacs language definitions"
```

---

### Task 9: Tutorial

**Files:**
- Create: `docs/tutorial/{00-intro,01-hello,02-values,03-control-flow,04-structs,05-patterns,06-errors,07-memory,08-strings,09-slices,10-concurrency}.md`
- Create: `docs/tutorial/run.sh`
- Create: `crates/otter-driver/tests/tutorial.rs`

**Interfaces:**
- Consumes: the completed language
- Produces: every code block in the tutorial is extracted, compiled, and run by `tutorial.rs`, so documentation cannot rot

- [ ] **Step 1: Write the failing test** that walks the tutorial directory, extracts ```otter blocks, compiles and runs each, and asserts the expected output comment matches.

- [ ] **Step 2: Run to verify they fail** → FAIL.

- [ ] **Step 3: Write the tutorial**, teaching the shape of the language in the order a reader needs it. Explain the no-null and no-exceptions decisions where a reader will wonder about them.

- [ ] **Step 4: Run to verify they pass** → PASS.

- [ ] **Step 5: The real acceptance test** — give the tutorial to a person who has not seen the language, with no help, and have them write and run a program using only the document. Record what confused them and fix the tutorial. This is the task's actual deliverable and cannot be automated; do not mark it done without doing it.

- [ ] **Step 6: Commit**

```bash
git add docs/tutorial/ crates/otter-driver/tests/tutorial.rs
git commit -m "docs: executable tutorial with automated example verification"
```

---

### Task 10: Close the ledger and the CI gate

**Files:**
- Modify: `docs/superpowers/plans/2026-10-16-otter-master-plan.md`
- Modify: `.github/workflows/ci.yml`

**Interfaces:**
- Consumes: the whole M7–M8 implementation
- Produces: ledger L1/L3 updated; CI also gating on `cargo test -p otter-fmt` and `-p otter-lsp` and running the tutorial test

- [ ] **Step 1: Reconcile the ledger with the code.**

- [ ] **Step 2: Extend CI** to run the fmt, LSP, and tutorial tests on both platforms.

- [ ] **Step 3: Push, confirm green, commit.**

```bash
git add docs/superpowers/plans/2026-10-16-otter-master-plan.md .github/workflows/ci.yml
git commit -m "docs: close the ledger for M7-M8 and gate tooling in CI"
```

---

## Definition of Done for M7–M8

- `pool.ot` and `pingpong.ot` are correct and terminate across ten consecutive runs on both platforms.
- A shared string under concurrent retain/release does not double-free, and an unshared string still uses the plain refcount path.
- `otter fmt` is idempotent, preserves every comment, and does not change a program's meaning.
- The language server publishes and clears diagnostics, and survives unparseable input.
- Every tutorial example compiles, runs, and matches its stated output.
- A person who has never seen Otter wrote and ran a program using only the tutorial.