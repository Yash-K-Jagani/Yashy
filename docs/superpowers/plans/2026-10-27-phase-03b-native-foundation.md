# Phase 3b — Native Foundation: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the language able to say what C says. Bitwise operators with checked shifts, an `extern` block that can rename a symbol and declare a variadic, `repr(C)` with `sizeof`/`alignof`/`offsetof`, byte-level buffers, and seven builtins that stop answering `""` or `0` for "it failed" — so that "safer than C at C's speed" is a claim about a language someone can write a program in.

**Architecture:** Three layers, each a small addition to an existing one. The **operators** extend `BinOp`/`UnOp` in `yashy-parser` and emit `and`/`or`/`xor`/`shl`/`lshr`/`ashr` in `yashy-codegen`; the checked shift is one comparison in `expr_gen.rs` driven by a `ProofFacts` entry the verifier already knows how to produce. The **C boundary** extends `Item::Extern` with two optional fields (`link_name`, `variadic`) and adds one item modifier (`repr(C)`); layout is a pure function in `yashy-codegen` that both `sizeof` folding and the struct emitter call, so there is one layout algorithm and not two. The **honest builtins** change seven entries in `Checker::builtin`'s table from `prim(Str)`/`prim(I64)` to `Result`/`Option`, and the runtime returns a tag alongside the value it already returns.

**Tech Stack:** `crates/yashy-lexer` (five new operator tokens, one `DotDotDot`), `crates/yashy-parser` (`ast::{BinOp,UnOp,ExternDecl,Item}`), `crates/yashy-sema` (`check_expr.rs` builtin table and operator typing, `ty.rs` for `io.Error`), `crates/yashy-verify` (shift-range fact), `crates/yashy-codegen` (`expr_gen.rs`, `ir.rs` layout), `crates/yashy-runtime` (C: byte reads, result tags), `crates/yashy-driver` (CLI surface unchanged).

**Spec:** `docs/superpowers/specs/2026-10-27-yashy-master-design.md` §3.2 (the native foundation), §2 rows 20–21. Index: `docs/superpowers/plans/2026-10-27-yashy-master-plan.md`. Predecessor: `2026-10-27-phase-03-speed-foundation.md`.

## Global Constraints

See the master index. The ones that bite here:

- **No implicit coercion.** `u8 & i64` is an error, not a promotion. Every mixed-width operation names its `as`.
- **No operator overloading.** These six operators are defined on integers and on nothing else; a user type never gets them.
- **Checked by default.** A shift whose count is not provably in range keeps its check; only a `ProofFacts` entry removes it.
- **One layout algorithm.** `sizeof`, `offsetof`, the struct emitter and the C round-trip test all call the same function.
- **No `#[...]` attribute syntax.** `repr(C)` is one keyword form on three item kinds. A second `repr` needs a written exception.
- **The breaking change is one commit.** The seven builtin signatures change together, with the tutorial and examples, and no deprecation shim.
- **Both operating systems or it is not done** — including the C round-trip, which must build with MSVC on Windows and cc on Linux.

## Written cap (this phase)

Carried from the spec and enforced by tests:

- **Six operators, five compound forms.** No `<<<`, no rotate, no `popcount`, no `leading_zeros` as operators. A rotate is `(x << n) | (x >> (64 - n))` and a popcount is a stdlib function.
- **`repr(C)` is not an attribute system.** One keyword form, three item kinds (`struct`, `object`, fieldless `enum`), no arguments. `repr(Rust)`, `repr(align(8))`, `repr(packed)` and `repr(transparent)` are out of scope.
- **No `extern "C"` block-level variadic.** The `...` is per declaration and only on an `extern` declaration.
- **No general FFI type system.** No `c_char` alias, no automatic string marshalling, no lifetime on a pointer parameter. A C string is `*const u8` and the program says so.
- **No `alloc` change.** `alloc(n)` keeps counting `int` slots; `alloc_bytes(n)` is the byte form. Changing `alloc` would silently reinterpret every existing program.

## Review Focus

1. **A shift that is checked in one place and masked in another is worse than either.** `x << 64` must panic with the operator's position on both OSes and on both signednesses, and there must be no code path — including constant folding in `const_eval` — that masks instead. The `const fn` evaluator and the runtime emitter call the same `shift_checked` helper.
2. **`repr(C)` must match the C compiler, not the spec.** The gate is a differential test: a Yashy program prints `sizeof`/`alignof`/`offsetof` for a generated `struct`, and a C program compiled with the platform's own compiler prints the same numbers for the same struct. A layout rule that is "obviously right" and differs by four bytes is the failure mode this test exists for.
3. **`offsetof` on a non-`repr(C)` type is rejected, not computed.** Yashy's own layout has no promise about padding, so an offset into it is a number that may change. `E0108` says so and names `repr(C)` as the fix.
4. **`link_name` must not become a second namespace.** The Yashy name resolves in Yashy's scope; the symbol name is what the linker sees. Two declarations with the same `link_name` and different signatures is `E0900`, because the linker will pick one and the program will not know which.
5. **The builtin change must not leave a sentinel anywhere.** After Task 7, `grep` for `read_file` in `crates/` must find no `prim(TypeKind::Str)` entry, and the runtime's `yashy_read_file` must return a tag. A partially-migrated builtin is the worst outcome: the signature says `Result` and the runtime still returns `""`.
6. **`read_line` at EOF and an empty line must differ.** This is the specific bug the change exists for; the test is a program that reads two lines from input containing one, and the transcript shows `Some`, then `None`.

---

### Task 1: Bitwise operators, lexed to emitted

**Files:**
- Modify: `crates/yashy-lexer/src/token.rs` (`Amp`, `Caret`, `Tilde`, `Shl`, `Shr`, and the compound forms `AmpEq`, `PipeEq`, `CaretEq`, `ShlEq`, `ShrEq`; `Pipe` already exists for closures and patterns, so `|=` is a new token and `|` keeps its meaning)
- Modify: `crates/yashy-lexer/src/lexer.rs` (longest-match: `<<=` before `<<` before `<=` before `<`)
- Modify: `crates/yashy-parser/src/ast.rs` (`BinOp::{BitAnd, BitOr, BitXor, Shl, Shr}`; `UnOp::BitNot`)
- Modify: `crates/yashy-parser/src/{exprs.rs,pratt.rs}` (precedence table)
- Modify: `crates/yashy-sema/src/check_expr.rs` (operator typing)
- Modify: `crates/yashy-sema/src/hir.rs` (`HirExpr::Binary` already carries `BinOp`; no new variant)
- Modify: `crates/yashy-codegen/src/expr_gen.rs` (`and`/`or`/`xor`/`shl`/`lshr`/`ashr`)
- Modify: `crates/yashy-common/src/codes.rs` (`BITWISE_NOT_INTEGER = "E0103"`, `BITWISE_MIXED_TYPES = "E0104"`), create `docs/errors/{E0103,E0104}.md`, modify `docs/errors.md`
- Modify: `crates/yashy-fmt/src/lib.rs` (spacing and precedence-preserving parentheses)
- Create: `crates/yashy-driver/tests/programs/bitwise.ya`, modify `crates/yashy-driver/tests/e2e.rs`

**Interfaces:**
- Produces: `&`, `|`, `^`, `~`, `<<`, `>>` on every integer type (`i8`…`i64`, `u8`…`u64`, `usize`) and on `bool` for `&`/`|`/`^` (non-short-circuit, distinct from `&&`/`||`), and on nothing else.
- Produces: precedence, loosest to tightest: `||` < `&&` < `|` < `^` < `&` < `==`/`!=` < `<`/`<=`/`>`/`>=` < `<<`/`>>` < `+`/`-` < `*`/`/`/`%`. This is C's table with Yashy's `&&`/`||` kept above the bitwise operators, so `a && b | c` parses as `(a && b) | c` and a reader who writes it gets a diagnostic suggestion to parenthesise.
- Produces: `E0103` when an operand is not an integer (or `bool` for the three logical bitwise forms), naming the operand's type.
- Produces: `E0104` when the two operands have different types, naming both and the `as` cast that would fix it. There is no implicit widening.
- Produces: `x &= y`, `x |= y`, `x ^= y`, `x <<= y`, `x >>= y` as statements; the compound form requires a `var` binding (`E0400` otherwise, the existing code).
- Produces: codegen uses `ashr` for a signed right shift and `lshr` for an unsigned one — the signedness is in the type, not in the operator.

- [x] **Step 1: Write the failing tests** — every operator on every integer type prints the expected value; `~0` on `i64` is `-1` and on `u8` is `255`; a signed `>>` of `-8` by 1 is `-4` (arithmetic) and an unsigned `>>` of `248u8` by 1 is `124` (logical); `1 << 3 | 1` is `9` (precedence); `~x` on a `float` is `E0103`; `1u8 & 1i64` is `E0104`; `x &= 3` on a `const` is `E0400`; the formatter round-trips `a & b | c ^ ~d` with the parentheses the parse implies.
- [x] **Step 2: Watch them fail.**
- [x] **Step 3: Implement** — the lexer's operator match is longest-first; the parser's Pratt table gains five binary and one prefix entry at the stated binding powers; the checker has one `integer_operands` helper that returns the common type or the diagnostic, and `bool` is admitted for the three logical forms only; codegen picks `shl`/`ashr`/`lshr` from `TypeKind` and the rest are direct.
- [x] **Step 4: Run** `cargo test -p yashy-lexer -p yashy-parser -p yashy-sema -p yashy-codegen -p yashy-fmt -p yashy-driver --test e2e` → PASS.
- [x] **Step 5: Commit** — `git commit -m "feat: bitwise operators with no implicit widening (E0103, E0104)"`

---

### Task 2: Checked shifts

**Files:**
- Modify: `crates/yashy-codegen/src/expr_gen.rs` (`gen_shift` emits a range check unless `ProofFacts` covers the count)
- Modify: `crates/yashy-codegen/src/ir.rs` (`shift_checked(value, count, bits, signed, span)` — one helper used by the runtime path, the `const fn` path and the interpreter-free constant folder)
- Modify: `crates/yashy-sema/src/const_eval.rs` (a constant shift out of range is the same panic, at compile time, with the same message)
- Modify: `crates/yashy-verify/src/pass/bounds.rs` (a range fact for the shift count: `0 <= n < bits`)
- Modify: `crates/yashy-common/src/codes.rs` (`SHIFT_OUT_OF_RANGE = "E0105"`), create `docs/errors/E0105.md`
- Create: `crates/yashy-driver/tests/programs/shift_checked.ya`, modify `crates/yashy-driver/tests/e2e.rs`
- Create: `crates/yashy-driver/tests/verify_corpus/shift_unproven.ya` (a shift count from `read_line().parse_i64()?` that is unproven and keeps its check)

**Interfaces:**
- Produces: `x << n` panics when `n >= bits(x)` or `n < 0`, with the message `panic: shift amount 64 is out of range for int (0..63)` at the operator's span.
- Produces: `ProofFacts` gains `ShiftInRange { span, bits }`, emitted by `yashy-verify` when the count's range domain is within `0..bits`. The check is removed only when the fact exists, exactly as bounds checks work in phase 3.
- Produces: a constant shift in `const fn` or a `const` initialiser is rejected at compile time with the same text and the operator's span, so a program cannot pass the check in one build mode and fail in another.

- [x] **Step 1: Write the failing tests** — `1 << 63` is `i64::MIN`; `1 << 64` panics with the range message and the operator's column; `1u8 << 8` panics; `1 << -1` panics; `const BAD = 1 << 64;` is the same panic at compile time; a shift by a literal `3` emits no check (assert on the IR text); a shift by a value from `parse_i64` keeps its check (assert the `icmp` is present); the verifier proves `n` in `0..8` from an `if n < 8` guard and the check disappears.
- [x] **Step 2: Watch them fail.**
- [x] **Step 3: Implement** — `shift_checked` builds the comparison, the branch to the panic call, and the shift; it takes `bits` from `TypeKind` rather than a constant, so `u8` and `usize` are not special cases. The verifier's fact is produced by the same interval code that proves array bounds; there is no second range analysis.
- [x] **Step 4: Run** `cargo test -p yashy-codegen -p yashy-sema -p yashy-verify -p yashy-driver --test e2e --test verify` → PASS.
- [x] **Step 5: Commit** — `git commit -m "feat: checked shifts, removed only by a verifier range fact (E0105)"`

---

### Task 3: `link_name` on extern declarations

**Files:**
- Modify: `crates/yashy-parser/src/ast.rs` (`ExternDecl { name, symbol: Option<String>, params, ret, variadic, unsafe_, span }`)
- Modify: `crates/yashy-parser/src/items.rs` (parse `link_name "sqrt"` after the signature)
- Modify: `crates/yashy-sema/src/check_expr.rs` (two names: the Yashy name in scope, the symbol in the extern table)
- Modify: `crates/yashy-sema/src/scope.rs` (extern symbol table)
- Modify: `crates/yashy-codegen/src/{module.rs,expr_gen.rs}` (declare and call by `symbol`, not by `name`)
- Modify: `crates/yashy-common/src/codes.rs` (`DUPLICATE_EXTERN_SYMBOL = "E0900"`, `MALFORMED_EXTERN = "E0901"`), create `docs/errors/{E0900,E0901}.md`
- Modify: `crates/yashy-fmt/src/lib.rs`
- Modify: `crates/yashy-driver/tests/programs/libc.ya` (call `sqrt` through `link_name`), modify `crates/yashy-driver/tests/e2e.rs`

**Interfaces:**
- Produces: `extern "C" link "m" { fn sqrt(x: f64) -> f64 link_name "sqrt"; }` — the Yashy name `sqrt` is what the program writes, the symbol is what the linker resolves.
- Produces: the default is unchanged — with no `link_name`, the symbol is the declaration's name.
- Produces: `E0900` when two declarations in one program resolve to the same symbol and disagree about the signature; two declarations that agree are allowed and emitted once.
- Produces: `E0901` when `link_name` appears on anything but an `extern` declaration, when it is not a string literal, or when it is empty.
- Produces: `stdlib/math.ya`'s `sqrt` keeps Newton's method. A Yashy `fn sqrt`
  and the C symbol `sqrt` would be one linker symbol, so the wrapper would call
  itself; the checker reports that as `E0900` instead of letting `clang` fail.
  Making the hardware call reachable from a Yashy function of the same name
  needs Yashy function symbols to be mangled, which is not part of this task.

- [x] **Step 1: Write the failing tests** — `sqrt(2.0)` through `link_name` matches the hardware result to the last bit; a `link_name` that collides with a different signature is `E0900`; the same signature twice is emitted once (assert on the IR: one `declare`); a symbol the program also defines is `E0900`; `link_name` on a Yashy `fn` is `E0901`; `link_name ""` is `E0901`; the formatter round-trips the form.
- [x] **Step 2: Watch them fail.**
- [x] **Step 3: Implement** — `ExternDecl` gains `symbol: Option<String>`; the checker inserts the Yashy name into the ordinary scope and records `(symbol, signature)` in a `BTreeMap` for the duplicate check and the emitter; codegen calls `symbol.unwrap_or(name)`. The IR emitter declares each symbol once, keyed by the symbol string.
- [x] **Step 4: Run** `cargo test -p yashy-parser -p yashy-sema -p yashy-codegen -p yashy-driver --test e2e` → PASS.
- [x] **Step 5: Commit** — `git commit -m "feat(ffi): link_name, so a declaration can resolve to a different symbol (E0900, E0901)"`

---

### Task 4: Variadic extern declarations

**Files:**
- Modify: `crates/yashy-lexer/src/token.rs` (`DotDotDot`; `DotDot` and `DotDotEq` already exist for ranges)
- Modify: `crates/yashy-parser/src/{ast.rs,items.rs}` (`ExternDecl.variadic: bool`; `...` only as the last parameter of an `extern` declaration)
- Modify: `crates/yashy-sema/src/check_expr.rs` (arity: at least the fixed count; every extra argument is `unsafe` and its type is the program's responsibility)
- Modify: `crates/yashy-codegen/src/module.rs` (the LLVM declaration carries `...`)
- Modify: `crates/yashy-common/src/codes.rs` (`VARIADIC_MISPLACED = "E0902"`), create `docs/errors/E0902.md`
- Modify: `crates/yashy-driver/tests/programs/libc.ya` (a `printf` call), modify `crates/yashy-driver/tests/e2e.rs`

**Interfaces:**
- Produces: `extern "C" { unsafe fn printf(fmt: *const u8, ...) -> i32; }`, called as `unsafe printf("%d %s\n", 42 as i64, "x")`.
- Produces: `E0902` when `...` appears on a Yashy `fn`, when it is not the last parameter, or when it appears in a function type (`fn(*const u8, ...) -> i32` is not a Yashy type).
- Produces: the checker requires at least the fixed parameters and accepts any number beyond; it does **not** type-check the variadic arguments, because C does not either. The call is `unsafe`, so the inventory in phase 2 lists it.
- Produces: integer variadic arguments must be written with their C width (`42 as i64`, not `42`), because there is no default promotion. The diagnostic for a bare `int` argument names the width.

- [ ] **Step 1: Write the failing tests** — `printf("%d\n", 42 as i64)` prints `42`; too few fixed arguments is `E0700` (the existing code); `...` on a Yashy `fn` is `E0902`; `...` not last is `E0902`; a variadic call without `unsafe` is the phase 2 `E1300`-family unsafe diagnostic; the IR declaration contains `...`.
- [ ] **Step 2: Watch them fail.**
- [ ] **Step 3: Implement** — the lexer adds `DotDotDot` and the longest-match order puts it before `DotDot`; the parser accepts it only inside an `extern` declaration's parameter list and only at the end; the checker's arity check is `args.len() >= fixed` when variadic; codegen appends `, ...` to the declaration.
- [ ] **Step 4: Run** `cargo test -p yashy-lexer -p yashy-parser -p yashy-sema -p yashy-codegen -p yashy-driver --test e2e` → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat(ffi): variadic extern declarations (E0902)"`

---

### Task 5: `repr(C)` and the layout queries

**Files:**
- Modify: `crates/yashy-lexer/src/token.rs` (`Repr` keyword; `KEYWORDS` grows to 32 - 28 at the start of the programme, plus phase 1a's `requires`/`ensures`/`invariant`, plus `repr` - and the phase 5 gate's count becomes 32)
- Modify: `crates/yashy-parser/src/{ast.rs,items.rs}` (`Item::Struct`/`Item::Object`/`Item::Enum` gain `repr: Option<ReprKind>`; `ReprKind::C` is the only variant)
- Modify: `crates/yashy-sema/src/check_expr.rs` (`sizeof`/`alignof`/`offsetof` as builtin const-folded calls)
- Modify: `crates/yashy-sema/src/ty.rs` (a type's layout is asked of codegen through one trait, so sema never computes it)
- Modify: `crates/yashy-codegen/src/ir.rs` (`fn layout(ty, repr) -> Layout { size, align, offsets: Vec<(String, usize)> }`; `repr_type` and `alignment_of` call it)
- Modify: `crates/yashy-common/src/codes.rs` (`REPR_NOT_ALLOWED = "E0106"`, `SIZE_OF_UNKNOWN = "E0107"`, `OFFSET_OF_NOT_REPR_C = "E0108"`), create `docs/errors/{E0106,E0107,E0108}.md`
- Modify: `crates/yashy-fmt/src/lib.rs`
- Create: `crates/yashy-driver/tests/programs/repr_c.ya`, `crates/yashy-driver/tests/programs/repr_c.c` (the C half of the differential test), modify `crates/yashy-driver/tests/e2e.rs`
- Create: `crates/yashy-driver/tests/verify_corpus/{repr_generic.ya,offsetof_plain.ya}`

**Interfaces:**
- Produces: `repr(C) struct P { a: u8, b: i64, c: u16 }`, `repr(C) object Q { .. }`, and `repr(C) enum E { A, B }` (fieldless only).
- Produces: `sizeof(P)` = 24, `alignof(P)` = 8, `offsetof(P, b)` = 8 on x86_64 SysV and on MSVC x64 — the numbers come from the platform's C ABI, and the test compares them with a C program compiled by the platform's own compiler rather than with a table.
- Produces: `sizeof`/`alignof`/`offsetof` are `usize`-typed constants usable in a `const` and in an array length; a non-constant argument is `E0107`.
- Produces: `E0106` for `repr(C)` on a generic type or on an `enum` with payloads, with the note that a C enum is fieldless and a C struct has no type parameters.
- Produces: `E0108` for `offsetof(T, f)` where `T` is not `repr(C)`, and for an unknown field, with the field list in the note.
- Produces: one `layout` function. `alignment_of` and `repr_type` in `ir.rs` are rewritten to call it, and the test that compares Yashy's own layout with the C one for a `repr(C)` type is the only place the two algorithms are compared.

- [ ] **Step 1: Write the failing tests** — the differential test above, on both OSes; `sizeof(u8)` is 1 and `alignof(i64)` is 8; `offsetof` on a non-`repr(C)` struct is `E0108`; `repr(C)` on `struct P[T]` is `E0106`; `repr(C) enum` with a payload is `E0106`; a `repr(C)` struct passed to a C function that reads its fields matches; the formatter round-trips `repr(C)`.
- [ ] **Step 2: Watch them fail.**
- [ ] **Step 3: Implement** — `layout` walks the fields in declaration order, aligns each to its own alignment, pads the total to the struct's alignment; for `repr(C)` it uses the C rules (which for these three types are the same walk with `enum` = 4 bytes), and for Yashy's own layout it uses the existing natural alignment. The `sizeof`/`offsetof` calls are folded in `const_eval` from that function, so there is no second computation.
- [ ] **Step 4: Run** `cargo test -p yashy-parser -p yashy-sema -p yashy-codegen -p yashy-driver --test e2e --test verify` → PASS, on Windows and Linux.
- [ ] **Step 5: Commit** — `git commit -m "feat(ffi): repr(C) with sizeof, alignof and offsetof (E0106-E0108)"`

---

### Task 6: Byte buffers

**Files:**
- Modify: `crates/yashy-runtime` (C: `yashy_alloc_bytes`, `yashy_read_bytes`, `yashy_write_bytes`)
- Modify: `crates/yashy-sema/src/check_expr.rs` (three builtins; `alloc_bytes(n: usize) -> *mut u8`, `read_bytes(path: string) -> Result[[]u8, io.Error]`, `write_bytes(path: string, body: []u8) -> Result[void, io.Error]`)
- Modify: `crates/yashy-codegen/src/expr_gen.rs` (a `[]u8` is an ordinary slice; the existing slice paths apply unchanged)
- Modify: `crates/yashy-sema/src/ty.rs` (register `io.Error` as a builtin fieldless enum: `NotFound`, `PermissionDenied`, `AlreadyExists`, `NotAFile`, `Other`)
- Modify: `crates/yashy-driver/tests/programs/libc.ya` or create `bytes.ya`, modify `crates/yashy-driver/tests/e2e.rs`
- Modify: `docs/cli.md` only if a flag changes (it does not)

**Interfaces:**
- Produces: `let buf = alloc_bytes(4096);` — 4096 bytes, not 4096 `int`s. `buf[0] = 0xFF;` works because `*mut u8` is a pointer to a byte.
- Produces: `read_bytes(path)?` returns a `[]u8` whose `len()` is the byte count and whose every existing slice method (`slice`, `contains`, `get`, `free`) applies.
- Produces: `write_bytes(path, data)?` writes the bytes and returns unit or an `io.Error`.
- Produces: an `int` index or length where a `usize` is required is `E0100` with a note naming the cast, not a silent conversion. `usize` is already a distinct type (`TypeKind::CSize`), so this is the existing rule, not a new one.

- [ ] **Step 1: Write the failing tests** — `alloc_bytes(4)` then write four bytes and read them back; a 1 MiB `read_bytes` round-trips through `write_bytes` and `read_bytes` with every byte equal; `read_bytes("/no/such/file")` is `Err(NotFound)`; `write_bytes` to a directory is `Err(NotAFile)`; `buf[0]` on a `*mut u8` is `u8`, and adding it to an `int` without a cast is `E0100`; the leak suite reports no leak when the buffer is freed and one when it is not.
- [ ] **Step 2: Watch them fail.**
- [ ] **Step 3: Implement** — `alloc_bytes` calls the same runtime allocator as `alloc` with a byte count; `read_bytes` opens in binary mode (`"rb"` / `_O_BINARY`) and returns the length and the tag; `write_bytes` writes exactly `len` bytes. `io.Error` is registered in the prelude's type table beside `Option` and `Result`, so `std.io` can name it without a circular import.
- [ ] **Step 4: Run** `cargo test -p yashy-sema -p yashy-codegen -p yashy-driver --test e2e --test leaks` → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat: byte buffers (alloc_bytes, read_bytes, write_bytes) with io.Error"`

---

### Task 7: Honest builtin signatures

**Files:**
- Modify: `crates/yashy-sema/src/check_expr.rs` (seven entries in the builtin table)
- Modify: `crates/yashy-runtime` (C: each of the seven returns a tag; `yashy_read_line` returns `Some`/`None`)
- Modify: `crates/yashy-codegen/src/expr_gen.rs` (`gen_try` already branches on the subject's own success tag after phase 5; the builtin results use the same `variant_tag` table)
- Modify: `crates/yashy-stdlib/yashy/{fs,env,os,io,time}.ya` (the documentation that describes the old sentinel behaviour)
- Modify: every program and example that calls one of the seven — `crates/yashy-driver/tests/programs/{wc,wc_words,json_count,json_keys,cwd,args}.ya`, `examples/*`
- Modify: `docs/tutorial/*` (the chapters that show `read_file`, `getenv`, `read_line`, `parse_i64`)
- Modify: `docs/errors/E0300.md` (the "missing `?`" page gains the builtin case)

**Interfaces:**
- Produces: the seven signatures from spec §3.2.4, in one commit:

  ```text
  read_file(path: string) -> Result[string, io.Error]
  write_file(path: string, body: string) -> Result[void, io.Error]
  getenv(name: string) -> Option[string]
  getcwd() -> Result[string, io.Error]
  read_line() -> Option[string]
  parse_i64(s: string) -> Option[int]
  parse_f64(s: string) -> Option[float]
  ```

- Produces: `?` works on all of them after phase 5's `Option` support, so `let line = read_line()?;` inside a function returning `Option[...]` is the idiom, and the tutorial shows it.
- Produces: no deprecation shim and no `_unchecked` variant. A program that ignores the result gets the existing "unused `Result`" treatment once phase 1b's warnings land: `E2000`-family, not a silent discard.
- Produces: the runtime's `yashy_read_line` distinguishes EOF from an empty line, which is the specific bug the change exists for.

- [ ] **Step 1: Write the failing tests** — a program reading two lines from one line of input prints `Some(...)` then `None`; `read_file` on a missing path is `Err(NotFound)` and on an empty file is `Ok("")` — the two are now different; `parse_i64("abc")` is `None` and `parse_i64("0")` is `Some(0)`; `getenv` of an unset name is `None` and of an empty value is `Some("")`; every existing program that used the sentinel is migrated and still prints its pinned output.
- [ ] **Step 2: Watch them fail.**
- [ ] **Step 3: Implement** — each runtime function gains an out-parameter or a tagged return; the checker's table entries change in one edit; `gen_try` is unchanged because phase 5 already made it read the subject's tag. The migration is mechanical: `?` where the function already returned the right type, `switch` where it did not.
- [ ] **Step 4: Run** `cargo test --workspace` and the tutorial, examples and leak harnesses → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat!: builtins report failure as Option/Result, never as \"\" or 0"`

---

### Task 8: Docs, diagnostics and the gate

**Files:** `docs/tutorial/{06-strings,08-io,10-ffi,13-errors}.md` (new or rewritten: bitwise, byte buffers, the C boundary, failure as a value), `docs/errors/{E0900,E0901,E0902,E0103,E0105,E0104,E0106,E0107,E0108}.md` (the nine pages, each with cause/example/fix), `docs/errors.md` (nine rows), `AGENTS.md` (identity: bitwise is checked; `repr(C)` is not an attribute system; a builtin never returns a sentinel), `llms.txt`, `docs/cli.md` (no new flags — say so).

- [ ] **Step 1: Write the chapters** with runnable blocks, including the C round-trip program from Task 5 as the worked example and the `read_line` EOF program from Task 7 as the failure-as-a-value example.
- [ ] **Step 2: Run the gate table on both OSes.**
- [ ] **Step 3: Commit and record** — `git commit -m "docs: the native foundation, and close phase 3b"`; update the index status and the ledger.

---

## Gate summary

| Gate | Command | Passes when |
|---|---|---|
| Bitwise total | `cargo test -p yashy-driver --test e2e` | `bitwise.ya` covers all six operators on all integer types, both signednesses of `>>`, and the precedence table |
| No implicit widening | `cargo test -p yashy-sema` | `u8 & i64` is `E0104`; `~float` is `E0103` |
| Shifts checked | `cargo test -p yashy-codegen -p yashy-driver --test e2e` | `1 << 64` panics at the operator; `const BAD = 1 << 64` is the same panic at compile time |
| A proof removes the check | `cargo test -p yashy-verify --test verify` | the guarded shift has no `icmp`; the unguarded one does |
| `link_name` | `cargo test -p yashy-driver --test e2e` | `sqrt` through `link_name` is bit-identical to the hardware result; `E0900`/`E0901` fire |
| Variadic | `cargo test -p yashy-driver --test e2e` | `printf("%d\n", 42 as i64)` prints `42`; `E0902` fires on the three misplacements |
| C layout matches C | `cargo test -p yashy-driver --test e2e` | `sizeof`/`alignof`/`offsetof` equal a C program's, on Windows and Linux |
| `repr(C)` rules | `cargo test -p yashy-driver --test verify` | generic type `E0106`; payload enum `E0106`; `offsetof` on a plain struct `E0108` |
| Bytes are bytes | `cargo test -p yashy-driver --test e2e --test leaks` | a 1 MiB round-trip is byte-identical; no leak with `.free()`, one without |
| Failure is a value | `cargo test --workspace` | no builtin returns `""`/`0` on failure; `read_line` distinguishes EOF from an empty line |
| No regression | `cargo test --workspace` | green on Windows and Linux |
