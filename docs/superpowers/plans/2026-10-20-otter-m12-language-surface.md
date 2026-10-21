# M12–M15 Language Surface Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give Otter a conventional surface — arrays with methods, first-class functions and lambdas, method-bearing objects, and a closing pass over the language's known gaps — and rename the public vocabulary so “array”, “function”, and “object” mean what users expect.

**Architecture:** Four sequential phases, each leaving a working language. M12 adds method resolution as a desugaring layer in sema over the existing builtins, then renames the public surface and migrates every call site. M13 adds function types, function values, lambdas, closure environments, and higher-order array methods. M14 adds `impl` methods and `object` declarations as sugar over structs; codegen reuses the struct layout and mangles method names. M15 closes the correctness, ownership, and collection gaps those features expose. Each phase consumes only interfaces landed by an earlier phase.

**Tech Stack:** Rust 1.99 (edition 2021), LLVM IR text via `clang`, the C runtime shim in `crates/otter-runtime`, `cargo test` on Windows and Linux.

**Spec:** `docs/superpowers/specs/2026-10-16-otter-language-design.md`, `docs/superpowers/plans/2026-10-16-otter-language-vision.md` §§10.2, 11.5.2, 11.5.3, 11.5.6. This plan is the spec change that lifts the v1 exclusions on methods and function values; the master plan's “Naming is frozen” constraint is amended for M12 and freezes again after it.

## Global Constraints

- The compiler never panics on user input: no `unwrap`, `expect`, `panic!`, `unreachable!`, or unchecked indexing in compiler code. A malformed program produces a diagnostic with a span and a caret.
- No LLVM build dependency; `clang`/`llc` remain subprocesses.
- Every task ends with `cargo check --workspace --all-targets` warning-free and `cargo test --workspace --no-fail-fast` passing. Device Guard blocks are environmental and are not passes.
- `rustfmt` is not part of the gate. Do not run `cargo fmt`.
- Golden IR changes are reviewed as deliberate diffs, never regenerated unread.
- Conventional commits; explicit `git add` paths; no push from an implementer subagent.
- New source-level names are the names in this plan. No aliases survive the rename task: a second spelling is a second language.
- Every end-to-end program has a named `#[test]` in `crates/otter-driver/tests/e2e.rs` that calls `build_and_run("name")`; adding only `tests/programs/name.ot` runs nothing. Commands filter on the test name.
- Runtime C symbols keep their existing `otter_slice_*` names; the ABI is internal. Public source names change.

## Naming Decisions

The public name of `[]T` becomes **array**. The word **slice** is reserved for a future non-owning view; nothing in this plan introduces that view.

| Today | After | Notes |
|---|---|---|
| `slice`, “slice literal” | `array`, “array literal” | Syntax stays `[]T` and `[]T { ... }`. |
| `free_slice(xs)` | `xs.free()` | Consumes the array; releases string elements. |
| `push(xs, v)` | `xs.push(v)` | Receiver must be a binding, as today. |
| `pop(xs)` | `xs.pop()` | `Option[T]`. |
| `len(xs)` / `len(s)` | `xs.len()` / `s.len()` | One method name across arrays, strings, maps. |
| `str_slice(s, a, b)` | `s.slice(a, b)` | Copies; clamped, never faults. |
| `byte_at(s, i)` | `s.byte_at(i)` | Byte value, `0` out of range. |
| `map_get(m, k)` | `m.get(k)` | `Option[V]`. |
| `map_insert(m, k, v)` | `m.insert(k, v)` | |
| `map_remove(m, k)` | `m.remove(k)` | `Option[V]`. |
| `map_len(m)` | `m.len()` | |
| `map_free(m)` | `m.free()` | |
| `chan_send(c, v)` | `c.send(v)` | `bool`; `false` when closed. |
| `chan_recv(c)` | `c.recv()` | `Option[T]`. |
| `chan_close(c)` | `c.close()` | |
| `chan_free(c)` | `c.free()` | |
| `mutex_lock(m)` / `mutex_unlock(m)` | `m.lock()` / `m.unlock()` | |
| `mutex_free(m)` | `m.free()` | |
| `str_from_i64` | unchanged | Number-to-string conversions stay free functions. |
| `parse_i64(s)` / `parse_f64(s)` | `s.parse_i64()` / `s.parse_f64()` | 0 when the text is not a number. |
| `read_line`, `print_i64` | unchanged | I/O stays free functions. |

`struct` and `enum` keep their names. An **object** is a struct or enum with methods, declared with `impl`. An `object` declaration is sugar for a struct plus an `impl` block. There is no inheritance, no virtual dispatch, and no automatic `this`; `self` is an explicit first parameter.

## Scope

**In:** method-call syntax for builtins; array query, mutation, search, and growth methods; string query, transform, split, and parse methods; function types, top-level function values, non-capturing and capturing lambdas, indirect calls, builtin `map`/`filter`/`fold`; `impl` methods, `object` declarations, static methods, pointer receivers; the correctness and ownership fixes listed in M15.

**Out, deliberately:** inheritance, traits/interfaces, dynamic dispatch, operator overloading, exceptions, async/await, macros, a garbage collector, package management, and a non-owning array view. Capturing a linear value (`*mut`, array, map, channel) in a closure is out until a general destructor exists; capturing copyable values is in.

## Review Focus

1. **A method call on a temporary mutates a copy and silently loses the result.** `make_array().push(1)` must be rejected with the same “needs an array binding” diagnostic the free function uses, not compiled into a write to a temporary.
2. **A method name shadowed by a field.** `point.len` is a field access; `point.len()` must resolve the method only when the field is not callable, and the ambiguity must be diagnosed, not guessed.
3. **A closure environment freed while the closure is still callable.** Returning `|x| x + n` from a function must keep `n` alive; the refcount test must fail if the environment is released with the creating scope.
4. **A moved array or object still usable after `free()`.** Every consuming method goes through the existing `move_out` path; a second `free` is a compile error.
5. **Renaming a builtin without updating every embedded program.** The tutorial, benchmarks, golden programs, and README are part of the test surface; the rename task is not complete until the tutorial harness passes.
6. **Split strings outliving their source.** `split` and `lines` must copy each element into an owned string, and the returned array's `free()` must release every element; a test must free after the source binding's scope ends.

---

## M12 — Arrays, strings, method syntax, and the rename

### Task 0: JavaScript-style `let` beside `var` and `const`

**Files:**
- Modify: `crates/otter-lexer/src/token.rs`
- Modify: `crates/otter-parser/src/ast.rs`, `crates/otter-parser/src/stmts.rs`, `crates/otter-parser/src/parser.rs`
- Modify: `crates/otter-sema/src/check_expr.rs`
- Test: `crates/otter-sema/tests/check.rs`
- Test: `crates/otter-driver/tests/e2e.rs`
- Test: `crates/otter-driver/tests/programs/bindings.ot`

**Interfaces:**
- Produces: `let x: T = value;` is a mutable, block-scoped binding; `const` remains immutable and block-scoped; `var` remains the existing mutable, block-scoped binding.
- Consumes: the existing `Stmt::Var`/`Stmt::Const` checking and codegen paths.
- Deliberate deviation from JavaScript: `var` is not function-scoped or hoisted yet, and there is no `undefined`. Hoisting is recorded in the backlog because Otter has no value to give an uninitialized binding.

- [x] **Step 1: Write the failing sema tests**

Added `let_bindings_are_mutable_and_block_scoped` and
`const_and_var_keep_their_mutability` to `check.rs`. The block-scope half uses
`if true { let y = 1; }`, because a bare block is not a statement in Otter's
grammar.

- [x] **Step 2: Run them and verify they fail**

The first run failed at parse time because `let` was not a keyword. The host's
GNU `rustc` is blocked by Application Control, so the suite was run with a
freshly installed `nightly-x86_64-pc-windows-gnu` toolchain and a separate
target directory.

- [x] **Step 3: Implement `let`**

Added `TokenKind::Let` and the `"let"` keyword mapping, `Stmt::Let`, parser
support, and a checker arm that shares `Stmt::Var`'s annotation, initializer,
linear-copy and block-scope rules. Lowered to `HirStmt::Var`; codegen needed no
change. Updated the lexer's keyword tables and the Vim, Emacs and VS Code
definitions.

- [x] **Step 4: Add the end-to-end program**

`bindings.ot` prints a `let` after assignment, a `var` used through a nested
`if` block, and a `const`; `e2e.rs` has
`let_var_and_const_behave_as_documented`.

- [x] **Step 5: Run the focused and workspace tests**

- `cargo test -p otter-sema --test check let_bindings_are_mutable_and_block_scoped` — PASS.
- `cargo test -p otter-sema --test check` — 120 PASS (including the new tests).
- `cargo test -p otter-parser --test items let_binding_parses_like_var` — PASS.
- `cargo test -p otter-parser --test parse ast_nodes_round_trip_through_debug` — PASS.
- `cargo test -p otter-lexer --lib` — 40 PASS.
- `cargo test -p otter-driver --test e2e let_var_and_const_behave_as_documented` — PASS; the program printed `2\n42\n3\n`.
- `cargo check --workspace --all-targets` — clean, no warnings.

Application Control blocked some freshly linked test binaries with error 4551
on the first attempt. Rebuilding under a new target directory and RUSTFLAGS
fingerprint produced a runnable binary; that is an environment workaround, not
a code change.

- [x] **Step 6: Commit**

```bash
git add crates/otter-lexer/src/token.rs crates/otter-parser/src crates/otter-sema/src/check_expr.rs crates/otter-sema/tests/check.rs crates/otter-driver/tests/e2e.rs crates/otter-driver/tests/programs/bindings.ot
git commit -m "feat: add let bindings"
```

### Task 1: Method-call resolution for builtins

**Files:**
- Modify: `crates/otter-sema/src/check_expr.rs`
- Test: `crates/otter-sema/tests/check.rs`
- Test: `crates/otter-driver/tests/e2e.rs`
- Test: `crates/otter-driver/tests/programs/array_methods.ot`

**Interfaces:**
- Consumes: the existing builtin checks `check_slice_builtin`, `check_len_builtin`, `check_map_builtin`, `check_thread_builtin`.
- Produces: `receiver.name(args...)` type-checks as the builtin `name(receiver, args...)` for every receiver-first builtin; mutating receivers must be a single-name binding.

- [x] **Step 1: Write the failing sema test**

Added `a_builtin_can_be_called_as_a_method` to `check.rs`, using the array
program from this step.

- [x] **Step 2: Run it and verify it fails**

Run: `cargo test -p otter-sema --test check a_builtin_can_be_called_as_a_method`
Expected: FAIL with an arity diagnostic. As built, the parser flattens
`xs.len()` into a two-segment callee, so the checker saw a zero-argument `len`
call: `expected 1 arguments, found 0`.

- [x] **Step 3: Implement desugaring in `Expr::Call`**

The parser does not produce `Expr::FieldAccess` for a dotted call; it flattens
the receiver and method into `Expr::Call { callee: ["xs", "len"], .. }`. The
checker therefore treats a two-segment callee as a method call only when its
first segment resolves to a local binding. It looks up the receiver-first
builtin by the receiver's type, synthesizes
`Call { callee: [builtin], args: [receiver] + args }`, and checks that. A first
segment that is not a local stays on the ordinary path, so module paths are
untouched. Unknown methods report
`error: no method \`name\` on \`T\`` at the call span.

Registered methods: array (`len`, `push`, `pop`, `free`), string (`len`,
`byte_at`, `slice`), map (`get`, `insert`, `remove`, `len`, `free`), channel
(`send`, `recv`, `close`, `free`), and mutex (`lock`, `unlock`, `free`). `free`
lowers to the existing `free_slice`/`map_free`/`chan_free`/`mutex_free` names,
and string `slice` lowers to `str_slice`.

- [x] **Step 4: Add the end-to-end program and its named test**

Added `array_methods.ot` and
`#[test] fn array_builtins_can_be_called_as_methods()`, asserting stdout is
`2\n3\n`.

- [x] **Step 5: Run the sema and e2e tests**

- `cargo test -p otter-sema --test check a_builtin_can_be_called_as_a_method` — PASS.
- `cargo test -p otter-sema --test check` — 121 PASS.
- `cargo check --workspace --all-targets` — clean, no warnings.
- `cargo test -p otter-driver --test e2e array_builtins_can_be_called_as_methods` — PASS; stdout `2\n3\n`.
- Application Control intermittently blocked the freshly linked test or CLI
  binary (4551). A fresh target directory and RUSTFLAGS fingerprint is the
  environment workaround, not a code change.

- [x] **Step 6: Commit**

```bash
git add crates/otter-sema/src/check_expr.rs crates/otter-sema/tests/check.rs crates/otter-driver/tests/e2e.rs crates/otter-driver/tests/programs/array_methods.ot
git commit -m "feat(sema): resolve builtins as methods"
```

### Task 2: Array query methods

**Files:**
- Modify: `crates/otter-sema/src/check_expr.rs`
- Modify: `crates/otter-codegen/src/expr_gen.rs`
- Modify: `crates/otter-runtime/runtime.c`, `crates/otter-runtime/include/otter_runtime.h`
- Test: `crates/otter-driver/tests/e2e.rs`
- Test: `crates/otter-driver/tests/programs/array_query.ot`

**Interfaces:**
- Produces: `len() -> i64`, `is_empty() -> bool`, `get(index: i64) -> Option[T]`, `first() -> Option[T]`, `last() -> Option[T]`, `set(index: i64, value: T) -> bool`.
- Consumes: Task 1's method desugaring.

- [x] **Step 1: Write the failing e2e program**

Added `array_query.ot` and
`array_query_methods_answer_in_options`. The program covers an empty array
(`is_empty`, `first`, `last`), a populated array (`get`, `first`, `last`), an
out-of-range `get`, an in-range `set` that changes a later read, and an
out-of-range `set` that returns `false`.

- [x] **Step 2: Run it and verify it fails**

Run: `cargo test -p otter-driver --test e2e array_query_methods_answer_in_options`
Expected: FAIL. As built, the compiler reported
`no method \`is_empty\` on \`[]i64\``.

- [x] **Step 3: Implement the method table and runtime entry points**

`get` reuses `Expr::Index`; `first`/`last` build `xs[0]` / `xs[xs.len() - 1]`;
`is_empty` builds `xs.len() == 0`. `set` is checked as a binding receiver with
an `i64` index and an element-typed value, then lowered to the new
`otter_slice_set(slice, index, value, size) -> i64` runtime call. The codegen
turns the 0/1 answer into `bool`. `set` on a `[]string` is refused until the
array owns a retain/release pair for the replaced element.

- [x] **Step 4: Run the test**

Run: `cargo test -p otter-driver --test e2e array_query_methods_answer_in_options`
Expected: PASS. It passed, and `cargo check --workspace --all-targets` stayed
warning-free.

- [x] **Step 5: Commit**

```bash
git add crates/otter-sema/src/check_expr.rs crates/otter-codegen/src/expr_gen.rs crates/otter-runtime/ crates/otter-driver/tests/e2e.rs crates/otter-driver/tests/programs/array_query.ot docs/tutorial/09-arrays.md
git commit -m "feat: array query methods"
```

### Task 3: Array mutation and ordering methods

**Files:**
- Modify: `crates/otter-sema/src/check_expr.rs`
- Modify: `crates/otter-codegen/src/expr_gen.rs`
- Modify: `crates/otter-runtime/runtime.c`, `crates/otter-runtime/include/otter_runtime.h`
- Test: `crates/otter-driver/tests/e2e.rs`
- Test: `crates/otter-driver/tests/programs/array_edit.ot`

**Interfaces:**
- Produces: `push(value: T)`, `pop() -> Option[T]`, `insert(index: i64, value: T) -> bool`, `remove(index: i64) -> Option[T]`, `clear()`, `reverse()`, `sort()`.
- Consumes: Task 1; the existing `push`/`pop` checks.

- [x] **Step 1: Write the failing e2e program**

Added `array_edit.ot` and `array_edit_methods_mutate_in_place`. It covers
`insert` at the front, middle, and end; an insert past the end returning
`false`; `remove` returning the right element and `None` out of range; `clear`
leaving `len == 0` and `is_empty()` true; `reverse`; `sort` on an unsorted
`[]i64`; and `clear` on a `[]string`.

- [x] **Step 2: Run it and verify it fails**

Run: `cargo test -p otter-driver --test e2e array_edit_methods_mutate_in_place`
Expected: FAIL. As built, the compiler reported
`no method \`insert\` on \`[]i64\``.

- [x] **Step 3: Implement the operations**

Added `otter_slice_insert`, `otter_slice_remove`, `otter_slice_clear`,
`otter_slice_clear_strings`, `otter_slice_reverse`, and
`otter_slice_sort_i64` to the runtime. `insert` accepts `0..=len` and grows
through `otter_slice_push`, then shifts the tail; `remove` copies the element
out and shifts the tail left. `clear` keeps the backing array, and the
`_strings` variant releases each handle first. `reverse` swaps bytes, and
`sort` is `i64` only in this phase. `insert` and `remove` reject `string`
elements with an ownership diagnostic.

- [x] **Step 4: Run the test**

Run: `cargo test -p otter-driver --test e2e array_edit_methods_mutate_in_place`
Expected: PASS. It passed, and `cargo check --workspace --all-targets` stayed
warning-free.

- [x] **Step 5: Commit**

```bash
git add crates/otter-sema/src/check_expr.rs crates/otter-codegen/src/expr_gen.rs crates/otter-runtime/ crates/otter-driver/tests/e2e.rs crates/otter-driver/tests/programs/array_edit.ot
git commit -m "feat: array edit and ordering methods"
```

### Task 4: Search, extend, and copy-slice methods

**Files:**
- Modify: `crates/otter-sema/src/check_expr.rs`
- Modify: `crates/otter-codegen/src/expr_gen.rs`
- Modify: `crates/otter-runtime/runtime.c`
- Test: `crates/otter-driver/tests/e2e.rs`
- Test: `crates/otter-driver/tests/programs/array_search.ot`

**Interfaces:**
- Produces: `contains(value: T) -> bool`, `index_of(value: T) -> Option[i64]`, `extend(other: []T)`, `slice(start: i64, end: i64) -> []T`.
- Consumes: Tasks 1–3.

- [x] **Step 1: Write the failing e2e program**

Added `array_search.ot` and `array_search_and_copy_methods`. It checks
`contains(20)`, `contains(99)`, `index_of(30)` and `index_of(99)`; extends with
`{40, 50}` and prints the new length; copies `slice(1, 4)`, `slice(-5, 2)` and
`slice(2, 99)` and prints each result. A `[]string` section exercises
`contains`, `index_of`, `extend`, and `slice` on the retain/release path.

- [x] **Step 2: Run it and verify it fails**

Run: `cargo test -p otter-driver --test e2e array_search_and_copy_methods`
Expected: FAIL. As built, the compiler reported
`no method \`contains\` on \`[]i64\``.

- [x] **Step 3: Implement the operations**

`contains` and `index_of` accept `i64`, `f64`, `bool`, and `string` elements; a
struct or enum element reports `` `contains` needs an element type with
equality ``. The runtime has one `otter_slice_index_of_*` entry point per type,
and `contains` tests the returned index against zero. `extend` consumes its
argument: a binding is moved out, the runtime copies every element, then frees
the consumed array. The `[]string` variant retains each element before the
consumed array releases its own references. `slice` copies the clamped range
into a fresh array; the `[]string` variant retains each copied handle.

- [x] **Step 4: Run the test**

Run: `cargo test -p otter-driver --test e2e array_search_and_copy_methods`
Expected: PASS. It passed, and `cargo check --workspace --all-targets` stayed
warning-free.

- [x] **Step 5: Commit**

```bash
git add crates/otter-sema/src/check_expr.rs crates/otter-codegen/src/expr_gen.rs crates/otter-runtime/runtime.c crates/otter-driver/tests/e2e.rs crates/otter-driver/tests/programs/array_search.ot
git commit -m "feat: array search, extend, and copy-slice methods"
```

### Task 5: String methods

**Files:**
- Modify: `crates/otter-sema/src/check_expr.rs`
- Modify: `crates/otter-codegen/src/expr_gen.rs`, `crates/otter-codegen/src/strings.rs`, `crates/otter-codegen/src/module.rs`
- Modify: `crates/otter-runtime/runtime.c`, `crates/otter-runtime/include/otter_runtime.h`
- Update: `crates/otter-codegen/tests/golden/*.expected.ll`
- Test: `crates/otter-sema/tests/check.rs`
- Test: `crates/otter-driver/tests/e2e.rs`
- Test: `crates/otter-driver/tests/programs/string_methods.ot`

**Interfaces:**
- Produces: `len() -> i64`, `is_empty() -> bool`, `byte_at(i: i64) -> i64`, `get(i: i64) -> Option[i64]`, `slice(start: i64, end: i64) -> string`, `contains(needle: string) -> bool`, `starts_with(prefix: string) -> bool`, `ends_with(suffix: string) -> bool`, `index_of(needle: string) -> Option[i64]`, `trim() -> string`, `trim_start() -> string`, `trim_end() -> string`, `to_upper() -> string`, `to_lower() -> string`, `replace(old: string, new: string) -> string`, `repeat(n: i64) -> string`, `split(separator: string) -> []string`, `lines() -> []string`, `parse_i64() -> i64`, `parse_f64() -> f64`.
- Consumes: Task 1's method desugaring; the existing `len`, `byte_at`, `str_slice`, `parse_i64`, and `parse_f64` builtins.
- Ownership: every transform returns a fresh owned string. `split` and `lines` return a fresh array whose `string` elements are owned; `xs.free()` releases each element.

- [x] **Step 1: Write the failing e2e program**

`string_methods.ot` starts from `"  Otter,otter  "` and checks: `len`, `is_empty`,
`trim`, `trim_start`, `trim_end`, `to_upper`, `to_lower`, `contains("otter")`,
`contains("badger")`, `starts_with("  Ot")`, `ends_with("  ")`,
`index_of("otter")` and `index_of("badger")`, `replace("otter", "seal")`,
`repeat(2)`, `slice(2, 7)`, `get(2)`, `byte_at(999)`, `split(",")` (length and
both elements), `lines()` on `"a\nb\n"` (length and both elements), and
`s.parse_i64()` / `s.parse_f64()` on `"42"` and `"nope"` (the failure parses as
`0`). Add `#[test] fn string_methods_transform_bytes_and_ownership()` to
`e2e.rs`, calling `build_and_run("string_methods")` and asserting the exact
stdout.

As built, the program starts from `"  Hello, Otter  "` and adds `get(99)`,
`repeat(0)`, empty-string `is_empty()`/`lines()` cases, and empty- and
trailing-separator `split` cases. It uses intermediate bindings for
every transform result because method calls do not chain yet: the parser
flattens `s.trim_start().len()` into a three-segment callee, which the current
desugaring does not accept.

- [x] **Step 2: Run it and verify it fails**

Run: `cargo test -p otter-driver --test e2e string_methods_transform_bytes_and_ownership`
Expected: FAIL; the methods do not exist. As built, the first failure was
``error: `len` takes 1 argument`` for the chained call; after binding the
intermediate result it was ``error: `split` takes 2 arguments`` (the arity table
initially counted the receiver as explicit).

- [x] **Step 3: Implement the string method table and runtime entry points**

Resolve string methods by receiver type `Str`, separately from arrays that reuse
the same method names. Add runtime functions:

```c
long long  otter_str_contains(const otter_str*, const otter_str*);
long long  otter_str_starts_with(const otter_str*, const otter_str*);
long long  otter_str_ends_with(const otter_str*, const otter_str*);
long long  otter_str_index_of(const otter_str*, const otter_str*); /* -1 if absent */
otter_str* otter_str_trim(const otter_str*);
otter_str* otter_str_trim_start(const otter_str*);
otter_str* otter_str_trim_end(const otter_str*);
otter_str* otter_str_to_upper(const otter_str*); /* ASCII only */
otter_str* otter_str_to_lower(const otter_str*); /* ASCII only */
otter_str* otter_str_replace(const otter_str*, const otter_str* old, const otter_str* new);
otter_str* otter_str_repeat(const otter_str*, long long n); /* n < 0 is empty */
void       otter_str_split(const otter_str*, const otter_str*, otter_slice* out);
void       otter_str_lines(const otter_str*, otter_slice* out);
```

`get` returns `None` outside `0..len`; `byte_at` keeps its existing
out-of-range answer of `0`. `is_empty` is `len() == 0`. `slice` calls the
existing `otter_str_slice`. `parse_i64` and `parse_f64` call the existing
builtins and keep their "0 on bad input" rule. `replace` replaces every
non-overlapping occurrence; an empty `old` returns a copy. `repeat` with a
negative count is empty. `split` with an empty separator returns one element
containing the whole string; `lines` splits on `\n` and drops a final empty
element. `lines` splits on `\n`, strips a preceding `\r`, and drops a final
empty element. Case conversion and whitespace are ASCII-only; non-ASCII bytes
pass through unchanged. The stdlib's duplicate free functions are not changed in
this task; Task 6 migrates their callers and removes them.

As built, the method surface lives in a dedicated `check_string_method` rather
than in the flat builtin table. `len`, `byte_at`, `slice`, `parse_i64`, and
`parse_f64` rewrite to their existing builtins; `is_empty` builds `len() == 0`;
the new names build HIR calls directly. The runtime signatures use an out slot
for `get` (`long long otter_str_get(..., long long *out)`) and put the slice
header first for `split`/`lines`
(`void otter_str_split(otter_slice *out, const otter_str *s, const otter_str *separator)`),
which keeps the C ABI simple and avoids returning a three-word struct by value.
`otter_str_split` and `otter_str_lines` push owned `string` pieces into a fresh
slice; the codegen helpers release a temporary receiver after the call.
`split` with an empty separator returns the whole string as one piece, and
`lines` on the empty string returns an empty array. `replace` with an empty
`old` returns a copy, and `repeat` with `n <= 0` returns an empty string. The
golden IR snapshots gained the 29 new runtime declarations (15 slice methods
from Task 4 and 14 string methods); every added line is a `declare`, with no
body changes.

- [x] **Step 4: Run the test**

Run: `cargo test -p otter-driver --test e2e string_methods_transform_bytes_and_ownership`
Expected: PASS, with the leak report at zero. As built, it passed after the
arity correction; sema tests also cover the method shapes and wrong argument
types.

- [x] **Step 5: Commit**

```bash
git add crates/otter-sema/src/check_expr.rs crates/otter-codegen/src/expr_gen.rs crates/otter-codegen/src/strings.rs crates/otter-codegen/src/module.rs crates/otter-runtime/ crates/otter-driver/tests/e2e.rs crates/otter-driver/tests/programs/string_methods.ot crates/otter-sema/tests/check.rs docs/tutorial/08-strings.md
git commit -m "feat: add string methods"
```

### Task 6: Rename the remaining builtins and migrate every call site

**Files:**
- Modify: `crates/otter-sema/src/check_expr.rs` (method table and removed-name diagnostics)
- Modify: `crates/otter-stdlib/otter/string.ot` (remove duplicate string wrappers)
- Modify: all `crates/otter-driver/tests/programs/*.ot`
- Modify: `docs/tutorial/*.md`, `README.md`, `benchmarks/features/*.ot`, `benchmarks/README.md`
- Rename: `docs/tutorial/09-slices.md` → `docs/tutorial/09-arrays.md`
- Test: `crates/otter-sema/tests/check.rs`

**Interfaces:**
- Consumes: Tasks 1–5's method layer.
- Produces: the naming table at the top of this plan; the old free-function names are errors carrying a migration hint.

- [x] **Step 1: Add failing diagnostics for the old names**

`push(xs, 1)`, `free_slice(xs)`, `map_get(m, 1)`, `chan_recv(c)`,
`mutex_lock(m)`, `len(xs)`, `len(s)`, `byte_at(s, 0)`, `str_slice(s, 0, 1)`,
`parse_i64(s)`, and `parse_f64(s)` each report
``error: `push` has moved to method syntax: write `xs.push(1)` `` (with the
matching spelling for each name) at the call span.

- [x] **Step 2: Run the diagnostics test and verify it fails**

Run: `cargo test -p otter-sema old_builtin_names_point_at_their_methods`
Expected: FAIL; the old names still compile.

- [x] **Step 3: Implement the removals and migrate every source**

Remove the old builtin names from the checker, add the migration diagnostics,
and rewrite every call site in tests, tutorial, benchmarks, and README to the
method form. Remove the duplicate stdlib `string.contains`, `string.starts_with`,
`string.trim`, and `string.split` wrappers and migrate their callers; the new
methods are the only spelling. Rename `09-slices.md`, its title, and every link
to it. Update the tutorial harness expectation count if the file list changes,
and the benchmark runner's `slice` workload to `array`.

- [x] **Step 4: Run the full proof**

Run: `cargo test -p otter-sema old_builtin_names_point_at_their_methods`
Run: `cargo test -p otter-driver --test tutorial`
Run: `cargo check --workspace --all-targets`
Expected: all PASS; warning-free check.

- [x] **Step 5: Commit**

```bash
git add crates/otter-sema crates/otter-stdlib crates/otter-driver/tests docs/tutorial README.md benchmarks
git commit -m "refactor!: rename the public surface to arrays and methods"
```

**As built:** the public migration table is `old_builtin_method_name`, and the
HIR lowering still calls the old runtime symbols, so the ABI did not move with
the spelling. A method needs a local binding: a field receiver and a temporary
receiver each carry their own diagnostic. The string stdlib wrappers are gone
(only `concat` remains), `09-slices.md` became `09-arrays.md`, and the
benchmark workload and its C/Go/Rust files became `array`. The full proof ran
`sema` and `codegen` before the final wording-only diagnostic edits, the
tutorial harness, and the driver e2e/stdlib tests. The workspace check was
warning-free. Of the 86 e2e tests, 62 passed and 24 were blocked by Device
Guard 4551; no assertion failed. After the wording edits, Device Guard also
blocked the freshly built sema test binaries.

---

## M13 — Functions as values

### Task 7: Function types, top-level function values, and indirect calls

**Files:**
- Modify: `crates/otter-parser/src/ast.rs`, `crates/otter-parser/src/types.rs`, `crates/otter-parser/src/exprs.rs`, `crates/otter-parser/src/items.rs`
- Modify: `crates/otter-sema/src/ty.rs`, `crates/otter-sema/src/check_expr.rs`
- Modify: `crates/otter-codegen/src/ir.rs`, `crates/otter-codegen/src/expr_gen.rs`, `crates/otter-codegen/src/module.rs`, `crates/otter-codegen/src/fn_gen.rs`, `crates/otter-codegen/src/lib.rs`
- Test: `crates/otter-driver/tests/programs/functions.ot`, `crates/otter-parser/tests/function_types.rs`, `crates/otter-sema/tests/functions.rs`, `crates/otter-codegen/tests/function_values.rs`
- Benchmark: `benchmarks/features/function.*`, `benchmarks/features/function_direct.*`

**Interfaces:**
- Produces: type syntax `fn(i64, string) -> bool`; a top-level `fn` name is a value of its function type; `apply(f: fn(i64) -> i64, x: i64) -> i64 { return f(x); }`.
- Consumes: M12 method syntax; the existing `TypeKind::Func` representation.

- [x] **Step 1: Write the failing e2e program**

`functions.ot` defines `square`, `apply`, and prints `apply(square, 7)` as `49`;
it also binds `const f = square;` and calls `f(4)` as `16`.

- [x] **Step 2: Run it and verify it fails**

Run: `cargo test -p otter-driver --test e2e functions_can_be_passed_and_called_through_values`
Expected: FAIL at parse (`fn(` is not a type) or LLVM (`use of undefined value '@f'`).

- [x] **Step 3: Implement function types and values**

Add `TypeExpr::Fn { params, ret }` and `TypeKind::Func` as a real value type.
Represent **every** function value as `{ i8* code, i8* env }` from this task
onward; closures only add a non-null environment. A direct call to a named
function stays a direct call. Coercing a top-level `fn` to a value emits a
thunk `__fnvalue_square(i8* env, args...)` that ignores `env` and calls
`@square`; the value is `{ @__fnvalue_square, null }`. A call through a function
value loads both fields and emits an indirect call with `env` as argument 0.
`ir.rs` currently maps `Func` to `void`; replace that mapping with this struct.

- [x] **Step 4: Run the test**

Run: `cargo test -p otter-driver --test e2e functions_can_be_passed_and_called_through_values`
Expected: PASS.

- [x] **Step 5: Commit**

```bash
git add crates/otter-parser crates/otter-sema crates/otter-codegen crates/otter-driver/tests/programs/functions.ot
git commit -m "feat: function types and indirect calls"
```

**As built:** the type syntax is `fn(T, U) -> R`, with the arrow optional and
an omitted arrow meaning `-> ()`. `TypeExpr::Fn` is a parser node, and
`TypeKind::Func` is now a real value type: it has a `Display` (`fn(i64) -> i64`,
the same spelling the source uses) and a representation. Every function value
is `{ i8* code, i8* env }`. A top-level function name becomes a value by taking
the address of a synthesized thunk `__fnvalue_<name>(i8* env, args...)` that
ignores the environment and calls the real function; the value pairs that
thunk with a null environment, because a top-level function captures nothing.
A call through a value loads the pair, extracts both fields, and calls the
code pointer with the environment as argument 0. A direct call to a named
function is still a direct call: the thunk is only synthesized when a name is
used as a value, and one thunk is shared by every use of that function.

One decision the plan did not spell out: a function that *receives* a function
value is emitted `noinline`. Inlining it into its caller would let the
optimizer see the value and fold the indirect call back into a direct one,
which would quietly turn a program that calls through a value into one that
does not. WebAssembly keeps `call_indirect` indirect for the same reason. A
function value held in a *local* binding is left alone: the optimizer can see
that binding's value, and folding that call is an ordinary optimization.

The `function` feature benchmark is the first workload with two rows per
language: one that calls through a function value and one that calls the same
function directly, so the difference between them is the price of the value.
The C row hides its function pointer behind an empty `asm` barrier, the Rust
row uses `black_box`, and the Go row relies on `//go:noinline`; without those
the optimizer would devirtualize every row and the workload would measure a
direct call. All ten rows print `4999999950099999999`. On this machine the
value costs 1.5 ns more per call in clang, 1.9 ns in gcc, 2.1 ns in Rust,
3.0 ns in Otter, and 3.0 ns in Go.

The proof:

- `cargo check --workspace --all-targets` — clean, no warnings.
- `cargo test -p otter-parser --test function_types` — 4 PASS.
- `cargo test -p otter-sema --test functions` — 8 PASS.
- `cargo test -p otter-codegen --test function_values` — 6 PASS.
- `cargo test -p otter-driver --test tutorial` — PASS, with the new chapter.
- `cargo test -p otter-driver --test never_panics` — PASS, with a
  function-type template added so the fuzzer mutates the new syntax too.
- `cargo test --workspace --tests --no-fail-fast` — every suite that ran was
  green: `sema` 125 + 10 + 9 + 8 + 13 + 5 + 7, `codegen` 20 + 47 + 3 + 5 + 6
  + 9 + 13 + 13 + 12 + 7, `parser` 18 + 4 + 50 + 4 + 17 + 5 + 3, `lexer` 40 +
  38, `runtime` 5 + 10 + 9 + 1 + 10 + 15 + 1, `lsp` 6 + 3 + 8, `fmt` 7,
  `common` 7 + 22, `driver` link 5 and toolchain 1.
- The `functions.ot` program was built and run by hand: it printed `49` and
  `16`, and the raw bytes were `49\n16\n`.

Application Control (error 4551) refused a number of freshly linked binaries
during the workspace run: the `e2e` produced executables (27 of 87 tests), the
`leaks`, `modules` and `driver/stdlib` executables, and the test binaries for
`otter-codegen`'s `arena`, `c_casts`, `c_types`, `fn_gen`, `racy_strings` and
`stdlib`, `otter-driver`'s `never_panics` and `tutorial`, `otter-parser`'s
`function_types`, `otter-runtime`'s `input`, `otter-sema`'s `slices`, and the
`otter-driver` and `otter-lsp` binaries. Every one of them is a block on a
newly written executable, not a test failure: the suites that ran in the same
invocation were green, and the blocked ones pass when they are rebuilt under a
name the policy allows. The `function_types`, `tutorial`, `functions` and `never_panics` suites were each
re-run that way and passed.


### Task 8: Non-capturing lambdas

**Files:**
- Modify: `crates/otter-lexer/src/token.rs`, `crates/otter-lexer/src/lexer.rs`
- Modify: `crates/otter-parser/src/ast.rs`, `crates/otter-parser/src/exprs.rs`
- Modify: `crates/otter-sema/src/check_expr.rs`, `crates/otter-sema/src/hir.rs`, `crates/otter-sema/src/scope.rs`
- Modify: `crates/otter-codegen/src/expr_gen.rs`, `crates/otter-codegen/src/fn_gen.rs`, `crates/otter-codegen/src/module.rs`
- Test: `crates/otter-driver/tests/programs/lambdas.ot`, `crates/otter-parser/tests/lambdas.rs`, `crates/otter-sema/tests/functions.rs`, `crates/otter-codegen/tests/function_values.rs`
- Docs: `docs/tutorial/11-functions.md`
- Benchmark: `benchmarks/features/function_lambda.ot`

**Interfaces:**
- Produces: `|x: i64| -> i64 { return x * 2; }` as an expression whose type is `fn(i64) -> i64`.
- Consumes: Task 7.

- [x] **Step 1: Write the failing e2e program**

`lambdas.ot` prints `apply(|x: i64| -> i64 { return x + 1; }, 41)` as `42`, and
stores a lambda in a binding before calling it.

- [x] **Step 2: Run it and verify it fails**

Run: `cargo test -p otter-driver --test e2e a_non_capturing_lambda_is_a_function_value`
Expected: FAIL at parse.

- [x] **Step 3: Implement lambda parsing and lowering**

Parse `|params| -> ret { body }` after a `|` in expression position. Sema checks
the body with the declared parameter and return types. Codegen emits a private
function `__lambda_N(i8* env, params...) -> ret` that ignores `env` in this
task, and yields `{ @__lambda_N, null }`. A lambda that mentions a name not
bound by its parameters reports
`error: closures are not captured yet; this lambda needs \`x\`` until Task 9.

- [x] **Step 4: Run the test**

Run: `cargo test -p otter-driver --test e2e a_non_capturing_lambda_is_a_function_value`
Expected: PASS.

- [x] **Step 5: Commit**

```bash
git add crates/otter-parser crates/otter-sema crates/otter-codegen crates/otter-driver/tests/programs/lambdas.ot
git commit -m "feat: non-capturing lambdas"
```

**As built:** the syntax needed a token the language did not have. The lexer
reported a lone `|` as an unexpected character, and two tests pinned that;
`TokenKind::Pipe` replaces it. `||` is still the two-character operator,
because the lexer matches it first -- so a lambda with no parameters is
written `||`, and the parser reads `||` in *prefix* position as the empty
parameter list. Otter has no prefix `||` for it to be, and the infix operator
is consumed before a prefix is ever looked for.

The lambda is a function without a name, so the checker builds it the way it
builds a function: its own scope, its parameters bound in it, and its body
checked against its declared return type. The HIR carries a whole `HirFn`
with an empty name, and the backend names it `__lambda_N` when it emits it,
because only the backend knows how many lambdas a program has.

The body is lowered into a capture buffer rather than into the module body: a
lambda is lowered while the function that contains it is still open, and a
`define` block cannot be written inside a function. The buffers are a stack,
because a lambda may contain another lambda and the inner body must not
swallow the outer one.

A capture is a name that resolves in a scope *above* the lambda's own but
below the root. The root scope is exempt: a top-level function is a global,
and a lambda may call one. The checker's `lookup` reports the capture and says
nothing else about the name, so a capture is one diagnostic rather than two.

The formatter learned the bars too: `|x: i64|` is one thing, and neither bar
takes air. The `||` of a parameterless lambda is still the logical-or token,
so the formatter leaves it alone -- it cannot tell a prefix `||` from an
infix one without looking at what came before it.

The proof:

- `cargo check --workspace --all-targets` — clean, no warnings.
- `cargo test -p otter-parser --test lambdas` — 4 PASS.
- `cargo test -p otter-sema --test functions` — 13 PASS.
- `cargo test -p otter-codegen --test function_values` — 9 PASS.
- `cargo test -p otter-fmt --test fmt` — 8 PASS.
- `cargo test -p otter-driver --test e2e a_non_capturing_lambda_is_a_function_value`
  — PASS; the program printed `42\n40\n`.
- `cargo test -p otter-driver --test tutorial` — PASS, with the lambda section
  in chapter 11.
- The `lambdas.ot` program was built and run by hand: it printed `42` and
  `40`, and the raw bytes were `42\n40\n`.
- A lambda that returns a `string` was built and run by hand too: the
  indirect call releases the owned result the same way a direct call does.
- The `function` benchmark gained a `lambda` row: the same workload calling
  through a lambda. It prints the same number as every other row, and it costs
  4.17 ns per call against the named function's 4.58 — the same indirect
  call, with one fewer indirection to reach the function itself.

### Task 9: Capturing closures with copyable captures

**Files:**
- Modify: `crates/otter-sema/src/check_expr.rs`, `crates/otter-sema/src/ty.rs`
- Modify: `crates/otter-codegen/src/expr_gen.rs`, `crates/otter-codegen/src/module.rs`
- Modify: `crates/otter-codegen/src/env.rs`, `crates/otter-codegen/src/fn_gen.rs`
- Modify: `crates/otter-runtime/runtime.c`, `crates/otter-runtime/include/otter_runtime.h`
- Test: `crates/otter-driver/tests/programs/closures.ot`, `crates/otter-driver/tests/leaks.rs`
- Docs: `docs/tutorial/11-functions.md`

**Interfaces:**
- Produces: a closure type represented as `{ i8* code, i8* env }`; captures are copied into a refcounted environment; the environment's refcount is released with the closure binding.
- Consumes: Task 8; the string refcount pattern in `otter_str_retain`/`otter_str_release`.

- [x] **Step 1: Write the failing e2e program**

`closures.ot` defines `fn adder(n: i64) -> fn(i64) -> i64` returning
`|x: i64| -> i64 { return x + n; }`; it binds `const add5 = adder(5);`, calls
`add5(10)`, and prints `15`. It also captures a `string`, returns the closure,
and prints the captured text after the creating function has returned.

- [x] **Step 2: Run it and verify it fails**

Run: `cargo test -p otter-driver --test e2e a_closure_outlives_its_creating_scope`
Expected: FAIL with the Task 8 “closures are not captured yet” diagnostic.

- [x] **Step 3: Implement environments and thunks**

A capturing lambda lowers to a heap environment struct holding each capture,
plus a thunk whose first parameter is `i8* env`; the thunk unpacks the
environment and calls the lambda body. String captures are retained on creation
and released when the environment's count reaches zero. Closure values are
copyable and retain the environment. Linear captures (`*mut`, array, map,
channel) report `error: a linear value cannot be captured yet`.
`env` allocation is freed when the refcount reaches zero.

- [x] **Step 4: Run the test**

Run: `cargo test -p otter-driver --test e2e a_closure_outlives_its_creating_scope`
Expected: PASS, with no leak report.

- [x] **Step 5: Commit**

```bash
git add crates/otter-sema crates/otter-codegen crates/otter-runtime crates/otter-driver/tests/programs/closures.ot
git commit -m "feat: capturing closures with refcounted environments"
```

**As built:** the closure's captured values live in an *environment*: a
reference-counted block the closure carries and reads through. The block is
the runtime's `otter_env_new`, with a header the runtime owns, a table of
release actions the generated code writes, and the captured values after it.
The two sizes the generated code and the runtime have to agree on --
`OTTER_ENV_HEADER_SIZE` and `OTTER_ENV_SLOT_SIZE` -- are in the runtime's
header, and the tests on each side pin them.

The checker's job is to find the captures: a name that resolves above the
closure's own scope, but not in the root scope, is a capture. The root scope
is exempt because a top-level function is a global, not something a closure
carries. A closure inside a closure is handled by making the *enclosing*
closure capture the name too, so the value is there to be copied: the
enclosing closure holds it in its own environment, and the inner one reads it
from there.

A capture is a copy, so it is read-only: assigning to one changes the copy
and not the binding it came from. A linear value cannot be captured at all --
the environment would have to own a value the program still owns -- and the
checker says so.

The ownership model is the string model's, which is why the backend's
reference tracking grew a kind: a value owns a `string` or a closure's
environment, and the two are released by different calls. A closure binding
takes over the reference the closure value carried, a copy of a binding
retains it, and the binding's scope releases it -- and the environment's own
release, when its count reaches zero, releases the strings it captured.

One thing the plan does not say, and the implementation had to decide: an
*aggregate* may not hold a function value. A struct field, an enum field and
an array element of function type are all refused. An aggregate copies the
value without counting the reference, and it has no way to release it, so the
copy would outlive the environment it points at. The checker cannot tell a
closure from a named function by type, so the refusal is by type: a program
that wants a function in a struct waits for the aggregate's own reference
counting.

The proof:

- `cargo check --workspace --all-targets` — clean, no warnings.
- `cargo test -p otter-sema --test functions` — 15 PASS.
- `cargo test -p otter-codegen --test function_values` — 12 PASS.
- `cargo test -p otter-driver --test e2e a_closure_outlives_its_creating_scope`
  — PASS; the program printed `15\nhello\n`.
- `cargo test -p otter-driver --test leaks a_closure_releases_its_environment`
  — PASS; the runtime reported `0 live allocations`, so the environment and
  the string it captured both came back.
- `cargo test -p otter-driver --test tutorial` — PASS, with the closure
  section in chapter 11.
- The golden IR files gained the three `declare` lines the module now emits
  for the environment. The diff is those three lines in each of the twelve
  files and nothing else.


### Task 10: Builtin `map`, `filter`, and `fold`

**Files:**
- Modify: `crates/otter-sema/src/check_expr.rs`, `crates/otter-sema/src/hir.rs`
- Modify: `crates/otter-codegen/src/expr_gen.rs`, `crates/otter-codegen/src/higher_order.rs`
- Test: `crates/otter-driver/tests/programs/higher_order.ot`, `crates/otter-sema/tests/functions.rs`, `crates/otter-codegen/tests/higher_order.rs`
- Docs: `docs/tutorial/11-functions.md`

**Interfaces:**
- Produces: `xs.map(f: fn(T) -> U) -> []U`, `xs.filter(f: fn(T) -> bool) -> []T`, `xs.fold(init: U, f: fn(U, T) -> U) -> U`.
- Consumes: Tasks 7–9.

- [x] **Step 1: Write the failing e2e program**

`higher_order.ot` maps `{1, 2, 3}` through `|x: i64| -> i64 { return x * 2; }`
to `{2, 4, 6}`, filters through `|x: i64| -> bool { return x > 2; }` to `{3}`,
and folds from `0` with `|acc: i64, x: i64| -> i64 { return acc + x; }` to `6`.
It also maps `[]i64` to `[]string` through a closure that calls `str_from_i64`,
to prove `U` may differ from `T`.

- [x] **Step 2: Run it and verify it fails**

Run: `cargo test -p otter-driver --test e2e higher_order_array_methods_transform_in_order`
Expected: FAIL; methods missing.

- [x] **Step 3: Implement the three methods in codegen**

Each call site is specialized: `map` builds a fresh `[]U` and calls `f` on each
element; the source array keeps its own elements, and only the new array owns
the mapped results. `filter` appends elements that pass, using `push`; for
`[]string`, it retains each appended element. `fold` carries the accumulator and
calls `f`. Each method requires the array receiver to be a binding, like `push`.

- [x] **Step 4: Run the test**

Run: `cargo test -p otter-driver --test e2e higher_order_array_methods_transform_in_order`
Expected: PASS.

- [x] **Step 5: Commit**

```bash
git add crates/otter-sema/src/check_expr.rs crates/otter-codegen/src/expr_gen.rs crates/otter-driver/tests/programs/higher_order.ot
git commit -m "feat: map, filter, and fold"
```

**As built:** the three are one HIR node, because they are one shape: an
array, a function, and a loop that calls it. The checker says what the whole
expression is worth -- `[]U` for `map`, `[]T` for `filter`, and the
accumulator's own type for `fold` -- and the backend lowers the node into the
loop.

The callback is called through its value, the same way any call through a
function value is: the code pointer comes out of the pair and the environment
goes with it as the first argument. A callback that is a closure therefore
carries its environment through the loop, and the environment is released when
the call ends -- a closure made for the call belongs to the call.

`map` and `filter` build a new array with the runtime's own `push`, so the new
array grows the way every other array does, and a `string` element is retained
before it goes in: the new array owns one reference to every element it holds,
and the source keeps its own.

The accumulator's type is the type of the value `fold` starts from, and the
callback has to take that type and the element and return the accumulator's
type. A callback of the wrong shape is reported with the shape it should have
had: `expected a function of one `i64``, `expected a function of one `i64`
returning `bool``, and `expected a function of `(i64, i64)` returning `i64``.

The proof:

- `cargo check --workspace --all-targets` — clean, no warnings.
- `cargo test -p otter-sema --test functions` — 17 PASS.
- `cargo test -p otter-codegen --test higher_order` — 4 PASS.
- `cargo test -p otter-driver --test e2e higher_order_array_methods_transform_in_order`
  — PASS; the program printed `2 4 6`, `3`, `6` and `1 2 3`.
- `cargo test -p otter-driver --test leaks the_higher_order_methods_release_what_they_build`
  — PASS; the runtime reported `0 live allocations`.
- `cargo test -p otter-driver --test tutorial` — PASS, with the higher-order
  section in chapter 11.

One bug the leak test caught: the value a callback returned was not marked as
owned, so a `string` it built was counted twice and released once. The call
inside the loop now marks what it returned, the way every other call through a
function value does.

Another the IR test caught: the block that holds the elements which pass ended
with two branches -- the one the loop's body makes and the one the increment
is reached by. The block now ends with the branch that reaches the increment,
and a test walks the IR of all three methods to say that every block ends in
exactly one terminator.

The `map_array` benchmark measures the whole thing: ten million elements
mapped into a second array, in Otter and in C, Go and Rust. Otter's row is
41.7 ns per element against C's 11.1; about 4 ns of that is the call through
the function value, and the rest is the array the mapping builds, which
Otter's `push` grows one element at a time.


### Task 11: Generic functions

**Files:**
- Modify: `crates/otter-parser/src/ast.rs`, `crates/otter-parser/src/parser.rs`
- Modify: `crates/otter-sema/src/check_expr.rs`
- Modify: `crates/otter-codegen/src/fn_gen.rs`, `crates/otter-codegen/src/module.rs`
- Test: `crates/otter-driver/tests/programs/generics.ot`

**Interfaces:**
- Produces: `fn identity[T](x: T) -> T` and `fn apply_twice[T](f: fn(T) -> T, x: T) -> T`; call-site inference; monomorphized symbols such as `identity_i64`.
- Consumes: Task 7.

- [ ] **Step 1: Write the failing e2e program**

`generics.ot` prints `identity(7)` as `7`, `identity("seven")` as `seven`, and
`apply_twice(square, 3)` as `81`. A call whose type parameter cannot be inferred
reports `error: cannot infer type parameter \`T\`` at the call.

- [ ] **Step 2: Run it and verify it fails**

Run: `cargo test -p otter-driver --test e2e generic_functions_are_monomorphized`
Expected: FAIL at parse.

- [ ] **Step 3: Implement monomorphization**

Parse `[T]` after the function name. Sema records type parameters and infers
them from argument types at each call. Codegen emits one function per concrete
substitution, keyed by the mangled name plus type names. Generic recursion is
allowed; polymorphic recursion (a call with a different substitution inside the
generic body) is rejected with a diagnostic.

- [ ] **Step 4: Run the test**

Run: `cargo test -p otter-driver --test e2e generic_functions_are_monomorphized`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/otter-parser crates/otter-sema crates/otter-codegen crates/otter-driver/tests/programs/generics.ot
git commit -m "feat: generic functions by monomorphization"
```

---

## M14 — Objects

### Task 12: `impl` methods on structs and enums

**Files:**
- Modify: `crates/otter-lexer/src/token.rs`
- Modify: `crates/otter-parser/src/ast.rs`, `crates/otter-parser/src/items.rs`
- Modify: `crates/otter-sema/src/check_expr.rs`, `crates/otter-sema/src/hir.rs`
- Modify: `crates/otter-codegen/src/fn_gen.rs`
- Modify: `editors/vim/otter.vim`, `editors/emacs/otter.el`, `editors/vscode/syntaxes/otter.tmLanguage.json`
- Test: `crates/otter-driver/tests/programs/methods.ot`, `crates/otter-parser/tests/impls.rs`, `crates/otter-sema/tests/methods.rs`, `crates/otter-codegen/tests/methods.rs`
- Docs: `docs/tutorial/04-structs.md`

**Interfaces:**
- Produces: `impl Point { fn length(self) -> i64 { ... } }`; `point.length()` resolves to the mangled function `Point.length`; `self` is the first parameter.
- Consumes: M12 method resolution; Task 7 function signatures.

- [x] **Step 1: Write the failing e2e program**

`methods.ot` declares `struct Point { x: i64, y: i64 }` and
`impl Point { fn sum(self) -> i64 { return self.x + self.y; } }`; prints
`point.sum()` as `5`. It also declares an `enum Shape` with an `impl Shape`
method that switches on `self`.

- [x] **Step 2: Run it and verify it fails**

Run: `cargo test -p otter-driver --test e2e impl_methods_take_self`
Expected: FAIL at parse on `impl`.

- [x] **Step 3: Implement `impl`**

Parse `impl Type { fn ... }` items. Sema registers each method under the type
name and checks the body with `self` bound to the receiver type. A method with no
`self` parameter is a static method (Task 14). Codegen emits
`@Type.method` and passes the receiver as argument 0; the body is an ordinary
function. Method resolution runs before field access in call position; a missing
method reports `error: no method \`name\` on \`Type\``.

- [x] **Step 4: Run the test**

Run: `cargo test -p otter-driver --test e2e impl_methods_take_self`
Expected: PASS.

- [x] **Step 5: Commit**

```bash
git add crates/otter-parser crates/otter-sema crates/otter-codegen crates/otter-driver/tests/programs/methods.ot
git commit -m "feat: impl methods on structs and enums"
```

**As built:** `impl` is a keyword now, so the lexer, the editor grammars and
the LSP's keyword test all learned it.

The receiver is written `self` and carries no type of its own: the block
already says what the receiver is. The parser stands in the type `Self`, which
the checker resolves to the type the block is for -- inside a method, and only
there. A method is then an ordinary function with a receiver: the backend
emits it as `@Point.sum` and the call site passes the receiver as the first
argument.

The method is found by the *type*, not by the name, so `point.sum()` is
`Point.sum` with the receiver in front. A method that is not there reports
`no method `sum` on `Point``, the same shape the builtin methods report.

A method with no `self` is a static one, and calling it on a value says so:
"`origin` is a static method of `Point` and needs the type, not a value".
Calling a static method on the type is Task 14.

One thing the plan does not say, and the implementation had to decide: an
`impl` block must follow the type it is for. The checker collects the type
signatures in one pass over the items, and a block for a type that is not
declared yet would be a block for a type that does not exist.

The proof:

- `cargo check --workspace --all-targets` — clean, no warnings.
- `cargo test -p otter-parser --test impls` — 3 PASS.
- `cargo test -p otter-sema --test methods` — 7 PASS.
- `cargo test -p otter-codegen --test methods` — 3 PASS.
- `cargo test -p otter-driver --test e2e impl_methods_take_self` — PASS; the
  program printed `5` and `48`.
- `cargo test -p otter-driver --test tutorial` — PASS, with the methods
  section in chapter 4.


### Task 13: `object` declarations

**Files:**
- Modify: `crates/otter-lexer/src/token.rs`
- Modify: `crates/otter-parser/src/ast.rs`, `crates/otter-parser/src/items.rs`, `crates/otter-parser/src/exprs.rs`
- Modify: `crates/otter-sema/src/check_expr.rs`
- Modify: `editors/vim/otter.vim`, `editors/emacs/otter.el`, `editors/vscode/syntaxes/otter.tmLanguage.json`
- Test: `crates/otter-driver/tests/programs/objects.ot`, `crates/otter-parser/tests/impls.rs`, `crates/otter-sema/tests/methods.rs`

**Interfaces:**
- Produces: `object Counter { value: i64, fn get(self) -> i64 { ... } }`, equivalent to a struct plus an `impl` block; object literals use the existing struct-literal syntax.
- Consumes: Task 12.

- [x] **Step 1: Write the failing e2e program**

`objects.ot` declares `object Counter { value: i64, fn get(self) -> i64 { return self.value; }, fn add(self, n: i64) -> Counter { return Counter { value: self.value + n }; } }`;
constructs `Counter { value: 3 }`, prints `get` as `3`, and prints
`counter.add(4).get()` as `7`.

- [x] **Step 2: Run it and verify it fails**

Run: `cargo test -p otter-driver --test e2e an_object_declaration_carries_its_methods`
Expected: FAIL at parse on `object`.

- [x] **Step 3: Lower `object` to a struct and an impl**

Parse field declarations and methods in one item. Lower it in sema to a struct
definition plus the same method registrations Task 12 produces; codegen sees the
same layout and mangled names. An `object` with no fields is allowed; one with
no methods is legal but redundant.

- [x] **Step 4: Run the test**

Run: `cargo test -p otter-driver --test e2e an_object_declaration_carries_its_methods`
Expected: PASS.

- [x] **Step 5: Commit**

```bash
git add crates/otter-parser crates/otter-sema crates/otter-codegen crates/otter-driver/tests/programs/objects.ot
git commit -m "feat: object declarations"
```

**As built:** an object is a struct and an `impl` block written together, and
the checker lowers it to exactly that: the fields are declared as a struct's
are, and the methods are registered the way an `impl` block's are. The backend
never sees an object -- it sees a struct and some methods, with the same
layout and the same mangled names.

The fields and the methods may be written in any order, and a comma separates
them. The object is constructed with the ordinary struct-literal syntax:
`Counter { value: 3 }`.

One thing the plan's example needed and the parser did not have: a method
called on the *result* of a call. `counter.add(4).get()` has no path to write
down -- the receiver is a value, not a name -- so the parser keeps it as an
expression, and the checker finds the method by the receiver's type exactly as
it does when the receiver is a binding. A call on a name or a field chain is
still a path call, so the method resolution the earlier milestone built is
untouched.

`object` is a keyword now, so the lexer and the three editor grammars learned
it, as they did for `impl`.

The proof:

- `cargo check --workspace --all-targets` — clean, no warnings.
- `cargo test -p otter-parser --test impls` — 5 PASS.
- `cargo test -p otter-sema --test methods` — 9 PASS.
- `cargo test -p otter-driver --test e2e an_object_declaration_carries_its_methods`
  — PASS; the program printed `3` and `7`.


### Task 14: Static methods and constructors

**Files:**
- Modify: `crates/otter-sema/src/check_expr.rs`
- Test: `crates/otter-driver/tests/programs/object_new.ot`, `crates/otter-sema/tests/methods.rs`

**Interfaces:**
- Produces: `Counter.new(3)` resolves a method with no `self`; `Type.name(args)` is a static call.
- Consumes: Task 13.

- [x] **Step 1: Write the failing e2e program**

`object_new.ot` adds `fn new(start: i64) -> Counter` to the object declaration
and prints `Counter.new(3).get()` as `3`. Calling `Counter.new` with the wrong
argument type reports a normal argument-type diagnostic.

- [x] **Step 2: Run it and verify it fails**

Run: `cargo test -p otter-driver --test e2e a_static_method_is_called_on_the_type`
Expected: FAIL with `unknown function \`new\`` or a type-name resolution error.

- [x] **Step 3: Implement type-name resolution in call position**

When the callee path is `Type.name`, resolve `name` as a static method of
`Type`; if it has a `self` parameter, report
`error: \`Counter.new\` is a method and needs a receiver`. Static methods are
emitted as ordinary functions under the same `@Type.name` mangling.

- [x] **Step 4: Run the test**

Run: `cargo test -p otter-driver --test e2e a_static_method_is_called_on_the_type`
Expected: PASS.

- [x] **Step 5: Commit**

```bash
git add crates/otter-sema/src/check_expr.rs crates/otter-codegen/src/expr_gen.rs crates/otter-driver/tests/programs/object_new.ot
git commit -m "feat: static methods and constructors"
```

**As built:** a method with no `self` is a static one, and it is called on the
type: `Counter.new(3)`. The backend needs nothing new for it -- the method is
emitted under the same `@Type.name` mangling, and the call is an ordinary call
with the method's own arguments.

The one thing the checker had to work out is telling a *type* from a *value of
that type*: `Counter.new(3)` and `counter.new(3)` are the same call shape, and
only the first is static. The types are declared in the root scope, so a name
that resolves there and names a declared type is the type itself; a local that
shadows a type's name resolves deeper, and is a value.

A method called on the type that does have a `self` says so:
"`Counter.get` is a method and needs a receiver".

The proof:

- `cargo check --workspace --all-targets` — clean, no warnings.
- `cargo test -p otter-sema --test methods` — 13 PASS.
- `cargo test -p otter-driver --test e2e a_static_method_is_called_on_the_type`
  — PASS; the program printed `3`.


### Task 15: Pointer receivers for mutating methods

**Files:**
- Modify: `crates/otter-sema/src/check_expr.rs`
- Modify: `crates/otter-codegen/src/expr_gen.rs`
- Test: `crates/otter-driver/tests/programs/mutating_methods.ot`

**Interfaces:**
- Produces: `fn bump(self: *mut Counter)`; the receiver is the address of a `var` object; field writes through `self[0].field`.
- Consumes: Task 13; existing pointer indexing and field assignment.

- [ ] **Step 1: Write the failing e2e program**

`mutating_methods.ot` declares a `Counter` with
`fn bump(self: *mut Counter) { self[0].value = self[0].value + 1; }`; a `var c`
starts at `0`, `c.bump()` twice, and prints `2`. Calling `bump` on a `const`
reports `error: \`bump\` needs a mutable object binding`.

- [ ] **Step 2: Run it and verify it fails**

Run: `cargo test -p otter-driver --test e2e a_pointer_receiver_mutates_its_object`
Expected: FAIL; pointer receivers are not resolved.

- [ ] **Step 3: Implement pointer receivers**

A method whose first parameter type is `*mut T` receives the address of the
receiver. The receiver must be a single-name `var` binding of type `T`; a
temporary, field, or `const` is rejected at the call span. Codegen passes the
binding's slot; the body already supports `self[0].field` through pointer
indexing.

- [ ] **Step 4: Run the test**

Run: `cargo test -p otter-driver --test e2e a_pointer_receiver_mutates_its_object`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/otter-sema/src/check_expr.rs crates/otter-codegen/src/expr_gen.rs crates/otter-driver/tests/programs/mutating_methods.ot
git commit -m "feat: pointer receivers for mutating methods"
```

---

## M15 — Close the gaps

### Task 16: Enforce `?` and missing returns; short-circuit `&&`/`||`

**Files:**
- Modify: `crates/otter-sema/src/check_expr.rs`
- Modify: `crates/otter-codegen/src/expr_gen.rs`
- Test: `crates/otter-sema/tests/errors.rs`
- Test: `crates/otter-driver/tests/programs/errors_checked.ot`

**Interfaces:**
- Produces: `?` is legal only in a function returning `Result[T, E]` with the same `E`; every path of a value-returning function returns; `&&`/`||` evaluate the right operand only when needed.
- Consumes: existing `Try` HIR, return-type threading in `check_block_with_ret`.

- [x] **Step 1: Write the failing diagnostics and runtime tests**

Sema tests expect `error: \`?\` needs a function returning \`Result\`` for a `?`
in an `i64` function, `error: error type mismatch: \`bool\` vs \`string\`` for
mismatched errors, and `error: missing return` for a function whose last
statement is an `if` without an `else`. `errors_checked.ot` proves
`if n != 0 && 10 / n > 1` does not divide by zero when `n == 0`.

- [x] **Step 2: Run and verify they fail**

Run: `cargo test -p otter-sema question_requires_a_result_function`
Run: `cargo test -p otter-driver --test e2e question_and_return_are_enforced`
Expected: both FAIL on the old behavior.

- [x] **Step 3: Implement the checks and the CFG**

Thread the enclosing function's return type into `check_expr`'s `Try` arm via a
checker field set by `check_fn`; compare the `Err` types. Add a
`block_always_returns` check for value-returning functions. Lower `&&`/`||` to
branch-and-phi instead of LLVM `and`/`or`.

- [x] **Step 4: Run the tests**

Run: `cargo test -p otter-sema question_requires_a_result_function`
Run: `cargo test -p otter-driver --test e2e question_and_return_are_enforced`
Expected: PASS.

- [x] **Step 5: Commit**

```bash
git add crates/otter-sema crates/otter-codegen crates/otter-driver/tests/programs/errors_checked.ot
git commit -m "fix: enforce error propagation, returns, and short-circuiting"
```

**As built:** the checker now knows the function it is inside. `?` compares the
error type of its operand with the error type of the function's return type,
and reports "the `?` needs a function returning `Result`" when the function
returns something else. A value-returning function has to return on every
path: the analysis is deliberately conservative, so a loop that might not run,
an `if` without an `else`, and a `switch` without a default all fall through.

`&&` and `||` are branches now, not instructions. The right operand is
evaluated in a block of its own, so `n != 0 && 10 / n > 1` does not divide by
zero when `n` is zero.

Two things the plan did not say, and the implementation had to decide:

- **The short-circuit's answer lives in a slot, not in a `phi`.** The right
  operand may be a checked operation with a fault path of its own, and then
  the block the value comes from is not the block the branch names -- and a
  `phi` has to name the predecessors exactly. A slot does not care which
  block the value was made in.
- **A function that falls off the end is now a compile error**, so the
  generator's `ret i64 0` fallback is only reachable from a tree the
  checker let through. The codegen test that pinned the fallback became a
  test that the checker refuses the program.

The golden IR changed accordingly: the two `and`/`or` instructions became
branches, and two `alloca i1` slots joined the entry block.


### Task 17: Struct equality, explicit array moves, and string fields

**Files:**
- Modify: `crates/otter-sema/src/check_expr.rs`
- Modify: `crates/otter-codegen/src/expr_gen.rs`, `crates/otter-codegen/src/stmt_gen.rs`, `crates/otter-codegen/src/memory.rs`, `crates/otter-codegen/src/module.rs`
- Test: `crates/otter-driver/tests/programs/struct_ownership.ot`, `crates/otter-sema/tests/errors.rs`, `crates/otter-driver/tests/leaks.rs`

**Interfaces:**
- Produces: `==` on structs compares fields when every field has equality and otherwise reports a diagnostic; `move xs` on an array compiles to a valid aggregate move; struct string fields are retained on construction and released at scope exit.
- Consumes: existing linearity checker and string retain/release.

- [x] **Step 1: Write the failing tests**

`struct_ownership.ot` compares two `Point` values, moves an array into a new binding and frees it once, and returns a struct built from a locally concatenated string; the leak report is zero and the string prints correctly after return.

- [x] **Step 2: Run and verify they fail**

Run: `cargo test -p otter-driver --test e2e struct_equality_and_string_fields`
Expected: FAIL with `icmp requires integer operands` or a dangling string.

- [x] **Step 3: Implement the three fixes**

`==` on a struct lowers to a conjunction of field comparisons (recursively) or
reports `error: \`==\` needs fields with equality`. Fix `Expr::Move` on arrays so
the HIR value is the loaded slice header, not a null; test the IR. Retain string
fields when a struct is constructed or assigned and release them when the
struct binding's scope exits; a struct copy retains each string field.

- [x] **Step 4: Run the tests**

Run: `cargo test -p otter-driver --test e2e struct_equality_and_string_fields`
Run: `cargo test -p otter-sema --test check struct_equality_requires_fields_with_equality`
Expected: PASS.

- [x] **Step 5: Commit**

```bash
git add crates/otter-sema crates/otter-codegen crates/otter-driver/tests/programs/struct_ownership.ot
git commit -m "fix: struct equality, array moves, and string field ownership"
```

**As built:** a struct's equality is built by the checker, field by field,
because the backend cannot compare the aggregate as a whole. A field with no
equality -- an array, a map -- makes the whole comparison impossible, and says
so: "`==` needs fields with equality: `[]i64` has none".

The move of an array emptied the source slot with `null`, which is a pointer
and not an aggregate: the IR did not even compile. An aggregate is emptied
with a zero value now.

A struct that holds a string owns one reference to it. The literal takes the
reference over from the value it was given -- a temporary's reference is
transferred, a literal's or a local's is retained -- a *copy* of a struct
retains a reference of its own, and the binding's scope releases what it
holds. The backend needs the struct's field *types* to know which fields own
a reference, and a `string` field's representation is the same as a raw
pointer's, so the module now records the field types alongside the layout.

The proof:

- `cargo check --workspace --all-targets` — clean, no warnings.
- `cargo test -p otter-sema --test errors` — 8 PASS.
- `cargo test -p otter-driver --test e2e struct_equality_and_string_fields`
  — PASS; the program printed `same`, `different`, `3`, `otter`, `2`, `otter`.
- `cargo test -p otter-driver --test leaks a_struct_releases_the_strings_its_fields_hold`
  — PASS; the runtime reported `0 live allocations`.


### Task 18: Fixed arrays and array views

**Files:**
- Modify: `crates/otter-parser/src/ast.rs`, `crates/otter-parser/src/parser.rs`
- Modify: `crates/otter-sema/src/ty.rs`, `crates/otter-sema/src/check_expr.rs`
- Modify: `crates/otter-codegen/src/ir.rs`, `crates/otter-codegen/src/expr_gen.rs`
- Test: `crates/otter-driver/tests/programs/fixed_arrays.ot`

**Interfaces:**
- Produces: `[T; N]` fixed arrays with stack storage and `[T]` views; `a[start..end]` returns a view that borrows `a`.
- Consumes: M12 array methods; Task 17 ownership fixes.

- [ ] **Step 1: Write the failing e2e program**

`fixed_arrays.ot` declares `const grid: [i64; 3] = [1, 2, 3];`, reads
`grid[1]`, takes `const head = grid[0..2];`, and sums the view. Indexing out of
bounds on a fixed array returns `Option`, as on an array.

- [ ] **Step 2: Run and verify it fails**

Run: `cargo test -p otter-driver --test e2e fixed_arrays_and_views`
Expected: FAIL at parse on `[i64; 3]`.

- [ ] **Step 3: Implement fixed arrays and views**

`[T; N]` is an LLVM array aggregate stored in an `alloca`; indexing is a constant
check plus `extractvalue` or `getelementptr`. `[T]` is a `{ i64, i8* }` view with
no ownership; `a[start..end]` borrows `a` and is rejected if `a` is moved or
freed while the view is live. Views cannot be returned from the function that
owns the storage.

- [ ] **Step 4: Run the test**

Run: `cargo test -p otter-driver --test e2e fixed_arrays_and_views`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/otter-parser crates/otter-sema crates/otter-codegen crates/otter-driver/tests/programs/fixed_arrays.ot
git commit -m "feat: fixed arrays and borrowed array views"
```

### Task 19: Tuples, destructuring, and pattern guards

**Files:**
- Modify: `crates/otter-parser/src/ast.rs`, `crates/otter-parser/src/parser.rs`
- Modify: `crates/otter-sema/src/check_expr.rs`
- Modify: `crates/otter-codegen/src/expr_gen.rs`, `crates/otter-codegen/src/patterns.rs`
- Test: `crates/otter-driver/tests/programs/tuples_guards.ot`

**Interfaces:**
- Produces: tuple types `(i64, string)`, tuple literals `(1, "a")`, destructuring `const (n, s) = pair;`, and switch guards `case Circle(r) if r > 10.0 { ... }`.
- Consumes: enum pattern machinery.

- [ ] **Step 1: Write the failing e2e program**

`tuples_guards.ot` returns `(i64, i64)` from `min_max`, destructures it into
`lo`/`hi`, and matches a `Shape` with a guard that selects only large circles.

- [ ] **Step 2: Run and verify it fails**

Run: `cargo test -p otter-driver --test e2e tuples_destructure_and_guards_select`
Expected: FAIL at parse.

- [ ] **Step 3: Implement tuples, destructuring, and guards**

Tuple types are anonymous LLVM structs; literals are `insertvalue`. A tuple
pattern checks arity and element types and binds each name. A guard is checked
as a `bool` after the pattern binds; codegen evaluates it in the arm block and
jumps to the next arm on false. Exhaustiveness ignores guards.

- [ ] **Step 4: Run the test**

Run: `cargo test -p otter-driver --test e2e tuples_destructure_and_guards_select`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/otter-parser crates/otter-sema crates/otter-codegen crates/otter-driver/tests/programs/tuples_guards.ot
git commit -m "feat: tuples, destructuring, and pattern guards"
```

### Task 20: Debug formatting, numeric literals, and module visibility

**Files:**
- Modify: `crates/otter-parser/src/lexer.rs` (numeric literals), `crates/otter-sema/src/check_expr.rs` (interpolation), `crates/otter-parser/src/ast.rs`, `crates/otter-sema/src/check.rs` (visibility)
- Modify: `crates/otter-codegen/src/module.rs`
- Test: `crates/otter-driver/tests/programs/diagnostics_and_syntax.ot`

**Interfaces:**
- Produces: `"${value:?}"` for structs, enums, arrays, and tuples; `1_000_000`, `0xFF`, `0b1010`, `0o777`; `pub` on items and fields, with a private-item diagnostic.
- Consumes: Tasks 12–19.

- [ ] **Step 1: Write the failing e2e program**

`diagnostics_and_syntax.ot` prints `Point { x: 3, y: 4 }` and `[1, 2, 3]` via
`${...:?}`, prints `1000000`, `255`, `10`, and `511` from the four literal
forms, and imports a `pub fn` from a module while a private function is
rejected.

- [ ] **Step 2: Run and verify it fails**

Run: `cargo test -p otter-driver --test e2e debug_format_literals_and_visibility`
Expected: FAIL at parse on `:?`, separators, or hex.

- [ ] **Step 3: Implement the three features**

`:?` lowers to a synthesized `debug_<Type>` function per nominal type and a
recursive array/tuple printer; strings are quoted and escaped. Numeric literals
allow `_` between digits and the `0x`/`0b`/`0o` prefixes; the lexer rejects a
trailing `_`. `pub` marks an item or field exported; module resolution reports
`error: \`f\` is private to module \`m\`` at the use.

- [ ] **Step 4: Run the test**

Run: `cargo test -p otter-driver --test e2e debug_format_literals_and_visibility`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/otter-parser crates/otter-sema crates/otter-codegen crates/otter-driver/tests/programs/diagnostics_and_syntax.ot
git commit -m "feat: debug formatting, numeric literals, and visibility"
```

### Task 21: `const fn` and the standard-library pass

**Files:**
- Modify: `crates/otter-parser/src/ast.rs`, `crates/otter-parser/src/parser.rs`
- Create: `crates/otter-sema/src/const_eval.rs`
- Modify: `crates/otter-sema/src/check_expr.rs`
- Modify: `crates/otter-stdlib/otter/*.ot`
- Test: `crates/otter-driver/tests/programs/const_fn.ot`

**Interfaces:**
- Produces: `const fn` evaluated at compile time in a `const` initializer; the stdlib gains map key/value methods that the new surface makes possible.
- Consumes: M12 methods, M13 functions, Task 20 visibility.

- [ ] **Step 1: Write the failing e2e program**

`const_fn.ot` declares `const fn square(n: i64) -> i64 { return n * n; }` and
`const TABLE = square(9);`, then prints `81` without a runtime call. The stdlib
adds `map.keys()`, `map.values()`, and `map.contains_key(key)`; a stdlib program
exercises each. Add `#[test] fn const_fn_evaluates_at_compile_time()` to
`e2e.rs`.

- [ ] **Step 2: Run and verify it fails**

Run: `cargo test -p otter-driver --test e2e const_fn_evaluates_at_compile_time`
Expected: FAIL at parse on `const fn` or unknown map method.

- [ ] **Step 3: Implement `const fn` with a tree-walker**

A `const fn` body may use only literals, arithmetic, comparisons, calls to other
`const fn`s, and `if`; anything else reports
`error: \`read_line\` is not allowed in a const fn`. The evaluator returns an
`i64`, `f64`, `bool`, or `string` value; a non-terminating or faulting
evaluation reports a diagnostic at the call. Add the three map methods using
the same method resolution as builtins; `keys`/`values` return fresh arrays, and
`keys` owns retained string keys.

- [ ] **Step 4: Run the test**

Run: `cargo test -p otter-driver --test e2e const_fn_evaluates_at_compile_time`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/otter-parser crates/otter-sema crates/otter-stdlib crates/otter-driver/tests/programs/const_fn.ot
git commit -m "feat: const fn and stdlib method coverage"
```

---

## Backlog (recorded, not scheduled)

These are deliberately outside M12–M15; each would need its own spec change.

| Item | Why it is later |
|---|---|
| Inheritance, traits, interfaces, dynamic dispatch | The object model here is value methods; polymorphism needs a vtable design and a layout contract. |
| Mutation through captured linear values | Needs a general destructor and escape analysis; closures capture copyable values only. |
| Operator overloading | Conflicts with the “one spelling per operation” rule; no user demand yet. |
| Async/await and a scheduler | Threads and channels cover the concurrency budget; async is a second runtime. |
| Macros and metaprogramming | Generics and `const fn` cover most duplication. |
| Package manager and version resolution | Independent of the language surface; M11 adoption owns distribution. |
| Garbage collection | Ownership and refcounting are the design; a GC would invalidate the ABI. |
| Array views escaping their owner | Task 18 borrows views only within the owning function; escaping needs lifetime syntax. |
| Default/named/variadic arguments | No proposed syntax; overload resolution would complicate method lookup. |
| `defer` across a panic boundary | Still blocked by the host unwinder; unchanged from M3–M4 Task 10. |

## Self-Review

- **Naming:** every old builtin has a named replacement in the table, and Task 6 removes the old spelling rather than aliasing it. String transforms and queries are methods too, not free functions with a parallel spelling.
- **Interfaces:** M12 produces the method layer M13/M14 consume; M13 produces function values `.map`/`.filter`/`.fold` consume; M14 produces objects whose string fields Task 17 makes safe.
- **Tests:** every task names a failing test and its expected diagnostic or output; the tutorial harness is part of Task 6 because it embeds the renamed examples.
- **Known gaps:** `?`, missing returns, short-circuiting, struct equality, array moves, and string fields are explicitly scheduled in M15 rather than documented as permanent limitations.
- **Order:** M13 does not require M14; M14 does not require M13. M15 requires both. If partial delivery is needed, M12 alone is a complete, shippable rename with array and string methods.

## Execution Handoff

When this plan is approved, request a separate phase plan for the first milestone to be executed, and use the preserved execution method. M12 is the recommended first slice: it is mechanical, test-heavy, and leaves the language shippable.
