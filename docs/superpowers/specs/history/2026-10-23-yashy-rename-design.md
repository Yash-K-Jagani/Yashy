# Yashy — Rename and Language-Surface Simplification

**Date:** 2026-10-23
**Status:** Draft for review
**Author:** Brainstormed with the project owner

---

## 1. Summary

Otter becomes **Yashy**. The rename is total: the language, the CLI, the file
extension, every crate, every runtime symbol, every environment variable, and
the live documentation. In the same change the language surface gets smaller:

- One integer type and one float type, spelled `int` and `float` (both 64-bit).
- A `char` type for one byte.
- `void` as the spelling for "returns nothing".
- One way to print: `print(x)` and `println(x)`, for every type.
- One way to make a reassignable binding: `let`.
- One way to release an owned value: `.free()`.
- Two array spellings instead of three: `[T; N]` and `[]T`.

The rename is deliberately separated from every semantic change, so the first
phase changes no behavior at all and the existing suite proves it.

### What exists today

| Thing | Count / spelling |
|---|---|
| Crates named `otter-*` | 10 |
| Programs in `*.ot` | 116 |
| Golden IR files | 12 |
| Test targets in the workspace | 76 suites |
| Runtime C symbols named `otter_*` | ~150 |
| Environment variables named `OTTER_*` | 17 |
| Scalar type spellings | `i64`, `f64`, `bool`, `string`, `()` (no value) |
| Print builtins | `print_str`, `print_i64`, `print_f64` (no bool, no char, no newline) |
| Binding forms | `const`, `let`, `var` (`var` duplicates `let`) |
| Free spellings | `free_slice(x)`, `map_free(m)`, `dealloc(p)` |
| Array/collection spellings | `[T; N]`, `[T]` (view), `[]T`, `map[K]V` |

Two mechanisms already exist and are reused rather than replaced:

- **Interpolation.** `"total = ${total}"` already lowers to concatenation, so
  `println("x = ${x}")` needs no new syntax.
- **Debug formatting.** Structs, enums, tuples, arrays, slices and the built-in
  sum types already render as `Point { x: 3, y: 4 }`, `Circle(2.5)`, `Empty`,
  `(1, true)`, `[1, 2, 3]`, `Some(7)`, `None`, `Ok(7)`, `"a\nb\"c"`.

---

## 2. Decisions of record

| Area | Decision |
|---|---|
| Language name | Yashy |
| Binary | `yashy` |
| File extension | `.ya` |
| Crates | `yashy-*` |
| Runtime symbols | `yashy_*` |
| Environment variables | `YASHY_*` |
| Integer type | `int`, 64-bit |
| Float type | `float`, 64-bit |
| Character type | `char`, one byte, 0–255 |
| No-value type | `void` |
| Printing | `print(x)`, `println(x)`; interpolation stays `${...}` |
| Reassignable binding | `let` only; `var` removed |
| Releasing a value | `x.free()` |
| Array type spellings | `[T; N]` and `[]T`; `[T]` views internal |
| Method receiver | `self` (unchanged) |
| Historical plan/spec files | keep their original wording |

### Calls made while designing

1. **Dated plans and specs are not rewritten.** They are records of what was
   decided when. Live docs change; the README carries one line explaining that
   the project was called Otter until 2026-10-23. Rewriting a dated record would
   make it say something it never said.
2. **`println()` with no arguments is legal** and prints one newline. It costs
   nothing and removes a special case from the explanation of the print family.
3. **`char` arithmetic requires `.to_int()`.** `'A' + 1` is an error; you write
   `'A'.to_int() + 1`. This is the language's existing "one spelling per
   operation" rule applied to the new type, and it keeps `char` from silently
   becoming an integer.

---

## 3. Naming and layout

- Rename the ten crates `otter-*` to `yashy-*`, the workspace package and binary
  `otter` to `yashy`, and every `*.ot` file to `*.ya`.
- Rename runtime symbols `otter_*` to `yashy_*` in `crates/yashy-runtime/*.c`
  and in every codegen string that names one.
- Rename environment variables `OTTER_*` to `YASHY_*` (`OTTER_LLC`,
  `OTTER_CACHE_DIR`, `OTTER_PATH`, `OTTER_TARGET`, `OTTER_HOST`,
  `OTTER_RUNTIME_H`, and the test-only ones) in code, scripts and docs.
- Rename the per-test temporary directories `otter-e2e-*`, `otter-*` in test
  harnesses, and the benchmark artifact names `*-otter.ll`.
- Prose: `Otter` becomes `Yashy`; lowercase identifiers `otter` become `yashy`;
  `OTTER` becomes `YASHY`.
- Tutorial code fences change from ` ```otter ` to ` ```yashy `; the harness that
  extracts them is updated with them.
- Editor grammars and the LSP test data follow the keyword/extension change.

Not renamed: dated files under `docs/superpowers/plans/` and
`docs/superpowers/specs/` other than this one. Their code samples keep `.ot`
and `i64`, because they are history, not instructions.

---

## 4. Types

- `int` (64-bit signed), `float` (64-bit), `string`, `bool`, `char` (one byte),
  `void`.
- `i64` and `f64` stop being spellings. Using one is an error that names the
  replacement: `` `i64` is spelled `int` in Yashy ``.
- `void` may be written only as a function's return type. `fn f()` and
  `fn f() -> void` mean the same thing; `let x: void`, `[]void` and
  `Option[void]` are errors, because a value of that type cannot exist.
- `()` stops being a type spelling. The no-value type is written `void`
  everywhere it may appear; `-> ()` is an error that names `void`. (Until now
  `()` was the spelling and `void` was C-only.)
- C widths (`i8`, `i16`, `i32`, `u8`, `u16`, `u32`, `u64`, `f32`, `usize`)
  remain available, but only inside `extern`, exactly as today. `void` leaves
  that C-only list and becomes an ordinary Yashy spelling, subject to the
  return-position rule above.
- Collections are unchanged: `[T; N]`, `[]T`, `map[K]V`, `Option[T]`,
  `Result[T, E]`, `Mutex`, `Channel[T]`.
- Internally, `TypeKind::I64`, `TypeKind::F64` and `TypeKind::Unit` keep their
  names. Renaming Rust enum variants would add churn to every match in the
  compiler without changing a single thing a user sees.

---

## 5. `char`

- Written `'A'`. Escapes: `'\n'`, `'\t'`, `'\r'`, `'\0'`, `'\''`, `'\\'`,
  and `'\x41'` (two hex digits, so 0x00–0xFF only).
- One byte, 0–255. A literal outside that range is a compile error.
- Distinct type: no implicit conversion to or from `int`, and no arithmetic.
  Comparing two `char` values with `==`, `!=`, `<`, `<=`, `>` or `>=` is
  allowed and uses byte order; comparing a `char` with an `int` is an error.
- Conversions: `c.to_int()` and `n.to_char()`. `n.to_char()` checks its input at
  run time and panics with a message naming the value when it is outside
  0–255, rather than truncating.
- String interop: `s.byte_at(i)` now yields a `char` (out of range still yields
  `'\0'`), and `c.to_string()` yields a one-character string. Strings are not
  iterable today (`for` iterates arrays only), and that does not change; a byte
  is reached with `s.byte_at(i)`.
- Printing: `print('A')` and `println('A')` write `A`. Inside interpolation,
  `"${c}"` writes the character, not its value.
- Storage is one byte in memory; `char` participates in structs, arrays,
  `Option[char]` and `Result[char, E]` like any other scalar.

---

## 6. Printing

- `print(x)` writes `x`; `println(x)` writes `x` and a newline; `println()`
  writes only a newline.
- One argument, any printable type. The checker resolves by the argument's type,
  so there is no format string and no varargs machinery.

| Argument type | Output |
|---|---|
| `string` | its bytes, unquoted |
| `char` | the character |
| `int` | decimal |
| `float` | existing float formatting (`2.5`, `0.5`) |
| `bool` | `true` / `false` |
| struct | `Point { x: 3, y: 4 }` |
| enum | `Circle(2.5)`, `Empty` |
| tuple | `(1, true)` |
| array / `[]T` | `[1, 2, 3]` |
| `Option[T]` / `Result[T, E]` | `Some(7)`, `None`, `Ok(7)`, `Err(...)` |

- Composites use the debug formatter; scalars use the plain form. This is the
  one distinction to learn: a top-level `string` prints raw, a string *inside*
  a composite prints quoted.
- The formatter already covers structs, enums, tuples, fixed arrays, slices and
  `Result`; nothing new is needed for composites.
- A `char` gains a display form (the character) and a debug form (`'A'`,
  quoted, so a newline in a `char` stays visible).
- Not printable: `map[K]V` (its order is not stable, so there is no honest
  text form without sorting), `Channel[T]`, `Mutex` and raw pointers. Passing
  one to `print`/`println` is a compile error that names the type, not a panic.
- Interpolation `${expr}` uses the same rules as `print`: scalars plain,
  strings raw, composites debug.
- Removed builtins: `print_str`, `print_i64`, `print_f64`. Each produces a
  diagnostic naming `print` or `println`. `str_from_i64`, `str_from_f64` and
  `str_from_bool` remain, because building a string is not printing it.
- `eprintln` is not added. Nothing in the current language writes to stderr
  except panics and assertions, and that stays true.

---

## 7. Simplifications

### 7.1 `var` is removed

`let` is the only reassignable binding. `const` still means "never reassigned".
`var` produces: `` `var` was removed: use `let` ``. Every program that uses
`var` is migrated to `let`.

### 7.2 One `.free()`

| Today | After |
|---|---|
| `free_slice(xs)` | `xs.free()` |
| `map_free(m)` | `m.free()` |
| `dealloc(p)` | `p.free()` |
| `arena_free_all(a)` | unchanged (it releases an arena, not one value) |

`.free()` is available on `[]T`, `map[K]V` and pointers. Calling it on a
`string` is an error, because strings are refcounted and release themselves at
scope exit. Freeing twice stays the type error it is today.

### 7.3 Two array spellings

- `[T; N]` is a fixed array, stored in the frame.
- `[]T` is a growable, owned array.
- `[T]` is no longer a type anyone can write. Writing it is an error:
  `` a view cannot be named: bind it with `let` and use it in place ``.
- The range expression `a[start..end]` still yields a borrowed range of `a`. It
  can be used in place (`grid[0..2].len()`) and bound with `let`, because the
  type is inferred. It cannot cross a function boundary, which was already true.
- No program in the repository names `[T]` today, so this removes a spelling
  without rewriting any program's logic.

---

## 8. What breaks, and how it is reported

Every removed spelling gets its own diagnostic rather than a generic parse
error, so an old snippet says what to do:

| Written | Diagnostic |
|---|---|
| `i64` | `` `i64` is spelled `int` in Yashy `` |
| `f64` | `` `f64` is spelled `float` in Yashy `` |
| `()` as a type | `` `()` is spelled `void` in Yashy `` |
| `var` | `` `var` was removed: use `let` `` |
| `print_str(x)` | `` `print_str` was replaced: use `println` (or `print`) `` |
| `print_i64(x)` / `print_f64(x)` | same |
| `free_slice(xs)` | `` `free_slice` was replaced: use `xs.free()` `` |
| `map_free(m)` | `` `map_free` was replaced: use `m.free()` `` |
| `dealloc(p)` | `` `dealloc` was replaced: use `p.free()` `` |
| `[T]` as a type | `` a view cannot be named: bind it with `let` and use it in place `` |

Each diagnostic carries a span and a caret, like every other diagnostic in the
compiler. Programs in the repository are migrated rather than left to trip these
messages; the diagnostics exist for readers of older material.

---

## 9. Verification

- `cargo check --workspace --all-targets` is warning-free.
- All 76 test suites pass. The launch-layer retry added for the host's
  Application Control policy stays; a refused launch is still never a pass.
- The tutorial still compiles and runs every block, printing exactly its
  `// =>` lines.
- Golden IR is regenerated once per phase that changes codegen, and the diff is
  reviewed to contain only the intended rename or surface change.
- Benchmarks are re-run after the semantic phases and their table updated; a
  phase that only renames must not move the numbers.
- New tests: the CLI reports `yashy version`; one diagnostic test per row of the
  table in section 8; `char` literals, escapes, conversions and printing;
  `print`/`println` for each row of the table in section 6, plus the error for
  a map; `.free()` on a slice, a map and a pointer, plus the error for a
  string; a test that `[T]` cannot be written; a test that `void` outside a
  return position is refused.

---

## 10. Order of work

Six phases. Each lands as its own commit series with the suite green, and no
phase depends on a later one:

1. **Rename only.** Crates, binary, extension, runtime symbols, environment
   variables, temp dirs, and every *name* mentioned in live docs (paths,
   commands, `.ot` references). Zero behavior change; this is where the suite
   proves the rename. The prose that describes the old surface is rewritten in
   phase 6, as each piece of surface changes.
2. **Type spellings.** `int`, `float`, `void`; `i64`/`f64`/`unit` diagnostics.
3. **`char`.** Lexer, checker, codegen, printing, string interop.
4. **Printing.** `print`/`println`, removal of the three print builtins,
   composite formatting through the debug formatter.
5. **Simplifications.** Remove `var`; `.free()`; hide `[T]`.
6. **Docs and benchmarks.** Tutorial, README, language reference, benchmark
   table, and the note about the former name.

---

## 11. Out of scope

- `self` stays (`this` was considered and declined).
- No implicit numeric conversions; `as` remains explicit.
- No Unicode `char`; that would drag UTF-8 decoding into indexing and is a
  separate decision.
- `object` stays as sugar for a struct plus its `impl`.
- No new collection types, no package manager work, no change to ownership.
- The repository's folder name and remote URL are not part of the change.

---

## 12. Risks

- **The rename is large but mechanical.** The risk is a missed occurrence in a
  string literal, a script, or a generated file. Mitigation: a repository-wide
  search for `otter`/`Otter`/`OTTER` must come back empty outside the dated
  plans, and the full suite must pass before the phase is committed.
- **Hiding `[T]` could surface a hidden dependency.** No `.ot` program names it,
  but a tutorial sentence and an inferred binding use it. Mitigation: phase 5
  rewrites the tutorial section and adds the "cannot be named" test.
- **Printing composites depends on the debug formatter being total.** Mitigation:
  the accepted set is the shapes the formatter already proves; a map, channel,
  mutex, pointer or function value is a compile error naming the type, never a
  panic.
