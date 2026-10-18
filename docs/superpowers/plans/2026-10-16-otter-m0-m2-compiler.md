# M0–M2 Implementation Plan — Otter compiler, part one

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a working Rust compiler front-end and LLVM text-emission backend that compiles Otter programs — integers, floats, bools, strings, structs, enums, pattern matching, `Option`, `Result`, and control flow — to native binaries.

**Architecture:** A Cargo workspace of seven crates, one per pipeline stage, with `otter-common` at the base owning byte spans and diagnostics. Source flows bytes → tokens → AST → typed HIR → LLVM IR text → `llc`/`clang` → native executable. Each crate is independently testable; `otter-sema` never emits code and `otter-codegen` never makes type decisions.

**Tech Stack:** Rust 1.99 (edition 2021), `cargo test`, `cargo-fuzz` for Task 14, LLVM `llc` (or `clang`) invoked as a subprocess. No parser-generator or AST-macro dependencies — hand-written recursive descent.

**Spec:** `docs/superpowers/specs/2026-10-16-otter-language-design.md` — especially §3 (surface), §4 (types), §10.2 (M0–M2 surface), §11 (testing). The spec travels with this plan; executors must read both.

## Global Constraints

- **Platform targets:** Windows x86-64 and Linux x86-64, from the first task. No platform-conditional language features.
- **Prerequisite — LLVM.** `llc` and `clang` are both absent on a fresh machine. Install with `winget install LLVM.LLVM`, then confirm a tool is reachable: `clang --version` on Windows — the Windows distribution ships **no `llc.exe`**, so `clang` is the expected result there and is the primary Windows path — or `llc --version` on Linux. Task 0 verifies this and fails loudly rather than working around it.
- **No LLVM build dependency.** The compiler crate tree must not link LLVM. LLVM is invoked only as a subprocess.
- **Diagnostics are v1, not v2** (spec §9.2). Every error carries a byte span and renders with a caret. No `panic!`, `unwrap()`, or `expect()` on user input anywhere in the pipeline — a malformed program produces a diagnostic and exit code 1.
- **Checked arithmetic does not land until M4** (spec §6.1, §10.2). At M0–M2 `i64` arithmetic emits LLVM wrapping ops and `sdiv`; division by zero is UB. Task 11 pins this interim behaviour with tests so the M4 change is deliberate.
- **`defer` lowers statically** at M0–M2 (spec §5.5): reverse-order emission before each scope exit. No unwinding machinery until M4.
- **Language surface is exactly spec §10.2.** Do not implement slices, maps, `alloc`, `move`, `extern`, `import` resolution, string operations, or interpolation in this plan. `import` parses and is rejected with a diagnostic until M6.
- **Commit format:** conventional commits, one commit per step-group, `git add` explicit paths.
- **Golden files** live in `crates/<crate>/tests/golden/`, named `<case>.ot` plus `<case>.expected`. Run with `cargo test -p <crate>`. An `UPDATE_GOLDEN=1` environment variable regenerates them; without it a mismatch fails.

## Review Focus

Failure modes the spec implies but which no milestone explicitly names. Each already has a test assigned below — if you write the task, write those tests.

1. **Division by zero at M0–M2.** `a / b` with `b == 0` emits `sdiv`, which LLVM treats as undefined; it can crash, or silently produce garbage under optimization. Task 11 pins interim wrapping semantics with a test that documents the hazard for M4.
2. **A program with no `fn main`.** Must produce a diagnostic naming the file, not an LLVM link failure 400 lines later. Task 10.
3. **Unterminated block — `fn f() {` with no closing brace.** Must produce an "unclosed brace" diagnostic whose caret points at **the opening brace**, with a message saying the file ended before the matching `}`, and never a lexer or parser panic. (An earlier draft said "a caret at end of file"; pointing at the opener is the more useful diagnostic and is what Task 4 implements — Task 14's fuzzing verifies it.)
4. **Duplicate definitions — two `fn main`, or a `const x` twice in one scope.** Must be a diagnostic, not last-one-wins. Task 8.
5. **Integer overflow at M0–M2.** `i64` add/mul emit wrapping LLVM ops. Under later optimization passes this must not silently become UB; Task 11's golden test asserts the exact opcode emitted.

---

## File Structure

```
Cargo.toml                     workspace root, members + resolver
rust-toolchain.toml            pins stable, so every contributor builds the same way

crates/otter-common/
  src/span.rs                  Span: byte offset range
  src/source.rs                SourceMap: path → text, line index, span→line:col
  src/diagnostic.rs            Diagnostic, Severity, DiagnosticBag, rendering
  tests/render.rs              caret rendering tests

crates/otter-lexer/
  src/lib.rs                   pub use re-exports
  src/token.rs                 TokenKind, Token, keyword table
  src/lexer.rs                 Lexer: bytes → Vec<Token>
  src/error.rs                 LexError → Diagnostic
  tests/lex.rs                 unit tests
  tests/golden/                .ot input + .expected token stream

crates/otter-parser/
  src/lib.rs
  src/ast.rs                   Span-carrying AST node types
  src/parser.rs                recursive descent, entry point
  src/items.rs                 items: import, struct, enum, fn
  src/stmts.rs                 statements
  src/exprs.rs                 expressions, precedence climbing
  src/error.rs                 ParseError → Diagnostic
  tests/parse.rs
  tests/golden/

crates/otter-sema/
  src/lib.rs                   check_program() entry point
  src/ty.rs                    Type, TypeKind
  src/hir.rs                   HIR node types + mutability
  src/scope.rs                 Scope, binding tables, duplicate detection
  src/check_expr.rs            type inference and checking
  src/check_pat.rs             pattern checking, exhaustiveness
  src/exhaustive.rs            exhaustiveness algorithm
  src/error.rs                 TypeError → Diagnostic
  tests/check.rs
  tests/exhaustive.rs
  tests/golden/                .ot + .expected HIR dump

crates/otter-codegen/
  src/lib.rs                   emit_program() entry point
  src/module.rs                LLVM module: globals, function bodies, string constants
  src/ir.rs                    IR value naming, LLVM type mapping
  src/fn_gen.rs                function emission, params, returns
  src/stmt_gen.rs              statement lowering, defer
  src/expr_gen.rs              expression lowering, operators
  src/patterns.rs              match lowering, Option/Result tag testing
  tests/codegen.rs
  tests/golden/                .ot + .expected .ll

crates/otter-driver/
  src/main.rs                  CLI entry
  src/toolchain.rs             finds llc/clang, diagnoses absence
  src/pipeline.rs              lex → parse → check → emit → link
  tests/e2e.rs                 end-to-end: .ot → native binary → assert output

crates/otter-runtime/
  runtime.c                    otter_print_str, otter_print_i64, otter_print_f64
  build.rs                     compiles runtime.c on demand via cc
  include/otter_runtime.h

fuzz/
  fuzz_targets/lex_parse.rs    never panics on arbitrary input
  Cargo.toml                   cargo-fuzz package
```

---

### Task 0: Workspace skeleton, runtime, and toolchain detection

**Files:**
- Create: `Cargo.toml`, `rust-toolchain.toml`
- Create: `crates/otter-common/{src/lib.rs,src/span.rs}`
- Create: `crates/otter-runtime/{runtime.c,build.rs,include/otter_runtime.h}`
- Create: `crates/otter-driver/{src/main.rs,src/toolchain.rs}`
- Create: `crates/otter-driver/tests/toolchain.rs`

**Interfaces:**
- Consumes: nothing (first task)
- Produces:
  - `otter_common::Span { pub start: u32, pub end: u32 }` — `Copy`, `Clone`, `Debug`, `PartialEq`, `Eq`, `Default` (default is `start == end == 0`, the span for synthetic nodes).
  - `otter_runtime::print_str(*const i8)`, `print_i64(i64)`, `print_f64(f64)` — exported C symbols used by all generated code.
  - `otter_driver::toolchain::Toolchain { pub llc: std::path::PathBuf }`
  - `otter_driver::toolchain::detect() -> Result<Toolchain, String>` — probes `OTTER_LLC`, then `llc`, then `clang` on `PATH`; the error string names each location tried.

- [ ] **Step 1: Create the workspace and confirm it builds**

`Cargo.toml` at the root, with `[workspace] resolver = "2"` and members listing all seven `crates/*` paths plus `fuzz`. Add `rust-toolchain.toml` pinning `channel = "stable"`.

Run: `cargo build`
Expected: succeeds with an empty workspace.

- [ ] **Step 2: Write the failing toolchain test**

In `crates/otter-driver/tests/toolchain.rs`:

```rust
#[test]
fn detect_reports_missing_toolchain_when_nothing_is_installed() {
    // Force the failure path by pointing every probe at a nonexistent binary.
    let err = otter_driver::toolchain::detect_with(|name| {
        if name == "llc" { Some(std::path::PathBuf::from("/nonexistent/llc")) } else { None }
    });
    assert!(err.is_err());
    let msg = err.unwrap_err();
    assert!(msg.contains("llc"), "error must name the binary it looked for: {msg}");
}
```

- [ ] **Step 3: Run the test to verify it fails**

Run: `cargo test -p otter-driver`
Expected: FAIL — `otter_driver` is not a library and `detect_with` does not exist.

- [ ] **Step 4: Implement `Span` and the runtime shim**

`otter-common/src/span.rs` holds `Span` as declared in Interfaces. `otter-runtime/runtime.c` implements the three print functions: `print_str` writes the NUL-terminated string to stdout; `print_i64` uses `%lld`; `print_f64` uses `%.17g`. `build.rs` compiles `runtime.c` with the `cc` crate and links it into `otter-driver`, so generated IR can call these symbols without a separate library. Add `otter-common` and `otter-driver` to the workspace with the other five crates stubbed as empty `src/lib.rs`.

- [ ] **Step 5: Implement `detect` and `detect_with`**

`detect_with` takes a `&dyn Fn(&str) -> Option<PathBuf>` probe so tests control the outcome without touching `PATH`. `detect()` calls `detect_with` with a probe that honours the `OTTER_LLC` environment variable first, then searches `PATH`. `clang` is accepted as a fallback because `clang -x ir` performs the same job as `llc`.

- [ ] **Step 6: Run the test to verify it passes**

Run: `cargo test -p otter-driver`
Expected: PASS.

- [ ] **Step 7: Install LLVM and verify end to end**

Run: `winget install LLVM.LLVM`
Then open a **new** shell (so `PATH` refreshes) and run `llc --version`.
Expected: prints an LLVM version banner. If this fails, stop — no later task can succeed without it.

- [ ] **Step 8: Commit**

```bash
git add Cargo.toml rust-toolchain.toml crates/
git commit -m "chore: workspace skeleton, runtime print helpers, LLVM toolchain detection"
```

---

### Task 1: Source map and diagnostic rendering

**Files:**
- Create: `crates/otter-common/src/source.rs`
- Create: `crates/otter-common/src/diagnostic.rs`
- Create: `crates/otter-common/tests/render.rs`

**Interfaces:**
- Consumes: `Span` from Task 0
- Produces:
  - `SourceMap::new() -> SourceMap`, `add_file(path: impl Into<String>, text: String) -> FileId`
  - `pub struct FileId(u32)` — `Copy`, `Debug`, `PartialEq`, `Eq`
  - `SourceMap::file(&self, id: FileId) -> &str`
  - `SourceMap::locate(&self, id: FileId, span: Span) -> Location` where `Location { line: u32, col: u32 }` is 1-based
  - `Severity { Error, Warning }`
  - `Diagnostic { pub severity: Severity, pub message: String, pub file: FileId, pub span: Span, pub label: Option<String> }`
  - `DiagnosticBag { diags: Vec<Diagnostic> }` with `push()`, `is_empty()`, `iter()`, `len()`, `into_vec()`
  - `SourceMap::render(&self, d: &Diagnostic) -> String`
  - `pub const EXIT_OK: i32 = 0`, `pub const EXIT_COMPILE_ERROR: i32 = 1`

- [ ] **Step 1: Write the failing rendering tests**

In `crates/otter-common/tests/render.rs`, three tests with these exact expectations:

```rust
#[test]
fn renders_caret_under_the_offending_column() {
    let mut sm = SourceMap::new();
    let f = sm.add_file("main.ot", "fn main() {\n    let x = 1;\n}\n".to_string());
    let d = Diagnostic {
        severity: Severity::Error,
        message: "cannot find value `y` in this scope".to_string(),
        file: f,
        span: Span { start: 18, end: 19 },
        label: None,
    };
    let out = sm.render(&d);
    assert!(out.contains("main.ot:2:7"), "got: {out}");
    assert!(out.contains("error: cannot find value `y` in this scope"), "got: {out}");
    assert!(out.contains("^"), "caret line missing: {out}");
}

#[test]
fn tab_in_source_does_not_shift_the_caret() {
    // A line indented with a tab must still place the caret at the right column.
    // Assert the caret column equals the source column, counted in characters.
}

#[test]
fn span_past_end_of_file_clamps_instead_of_panicking() {
    // A span reaching beyond the file must render the last line, not panic.
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p otter-common`
Expected: FAIL — `SourceMap` and `Diagnostic` do not exist.

- [ ] **Step 3: Implement `SourceMap` and `Diagnostic`**

`add_file` stores text and an index of newline positions. `locate` binary-searches the newline index for `span.start`, then counts characters from the line start for the column. Rendering writes, in order: `path:line:col`, the severity and message, the source line with tabs preserved, and a caret line. Clamp every offset into the file so no input can panic.

**Column arithmetic, pinned:** for the fixture above, byte 18 is the `t` of `let`. Line 2
starts at byte 12 and the indent occupies bytes 12–15, so the caret column is **7**, and
the rendered location is `main.ot:2:7`. An earlier draft of this plan asserted `2:5`,
which is arithmetically wrong for a four-space indent. Columns count characters from 1,
tabs count as one column, and a `\r` before a `\n` is part of the line ending and is not
rendered.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p otter-common`
Expected: PASS — 3 tests.

- [ ] **Step 5: Commit**

```bash
git add crates/otter-common/
git commit -m "feat(common): source map and caret diagnostic rendering"
```

---

### Task 2: Lexer

**Files:**
- Create: `crates/otter-lexer/src/token.rs`
- Create: `crates/otter-lexer/src/lexer.rs`
- Create: `crates/otter-lexer/src/error.rs`
- Create: `crates/otter-lexer/src/lib.rs`
- Create: `crates/otter-lexer/tests/lex.rs`
- Create: `crates/otter-lexer/tests/golden/{numbers,strings,comments,punct}.ot`

**Interfaces:**
- Consumes: `Span`, `Diagnostic`, `DiagnosticBag`, `Severity` from Tasks 0–1
- Produces:
  - `TokenKind` — unit variants for keywords `Fn Struct Enum Const Var Import Extern Unsafe Defer Return Break Continue If Else For In Switch Case Default As Move True False`; punctuation `LParen RParen LBrace RBrace LBracket RBracket Comma Semi Colon Dot Arrow At Plus Minus Star Slash Percent Eq EqEq BangEq Lt LtEq Gt GtEq PlusEq MinusEq StarEq SlashEq PercentEq`; `Ident`, `I64`, `F64`, `Str`, `Eof`
  - `As` and `Move` are keywords because spec §3.1 lists them and Task 6 parses `as` casts. An earlier draft of this plan's list omitted them, which would have made `x as f64` and `move buf` lex as plain identifiers. The list above is authoritative.
  - `@` lexes as `TokenKind::At` rather than an error. No Otter construct in M0–M8 produces it; it is kept because the interface list names it, and it will remain an unmatchable variant until an attribute syntax is designed.
  - `Token { pub kind: TokenKind, pub span: Span }`
  - `TokenKind::is_keyword(&self) -> bool`, `is_eof(&self) -> bool`
  - `Lexer::new(src: &str) -> Lexer`
  - `Lexer::tokenize(&mut self) -> Result<Vec<Token>, Vec<Diagnostic>>`
  - `TokenKind::describe(&self) -> String` — the spelling used in diagnostics
  - `StringLit { pub value: String, pub bytes: Vec<u8> }`
  - `pub fn literal_as_i64(src: &str, span: Span, file: FileId) -> Result<i64, Diagnostic>` — the **only** supported way to turn an `I64` token's text into a value, producing a diagnostic for a literal that does not fit. The `FileId` is **required, not optional**: a two-argument form silently attributes the diagnostic to file zero, which puts the caret in an unrelated source file — worse than no diagnostic. `TokenKind::I64` carries no value, so without this helper a downstream stage is one `parse::<i64>().unwrap()` away from panicking on a 40-digit literal. Task 4 must use it and must not parse integer literals by any other route.

- [ ] **Step 1: Write the failing unit tests**

In `crates/otter-lexer/tests/lex.rs`:

```rust
#[test]
fn lexes_i64_and_f64_literals_with_spans() {
    // "42" -> I64 at span 0..2; "3.5" -> F64 at 3..6.
    // Assert exact kinds and spans.
}

#[test]
fn lexes_keywords_as_keywords_not_identifiers() {
    // "fn" -> TokenKind::Fn; "fnx" -> TokenKind::Ident.
}

#[test]
fn lexes_line_and_nested_block_comments() {
    // "/* a /* b */ c */ 1" -> one I64 token at the right offset.
}

#[test]
fn reports_unterminated_string_with_the_opening_quote_span() {
    // "\"abc" -> Err containing one Diagnostic whose span starts at the quote.
}

#[test]
fn reports_unexpected_character_instead_of_skipping_it() {
    // "@" alone at EOF -> Err, not silently dropped.
}

#[test]
fn every_delimiter_is_reported_at_eof() {
    // For each of '(', ')', '{', '}', '[', ']' in isolation: unclosed-delimiter
    // diagnostic at EOF, never a panic.
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p otter-lexer`
Expected: FAIL — `Lexer` does not exist.

- [ ] **Step 3: Implement `TokenKind` and the keyword table**

`token.rs` holds the enum, `describe`, and a `match`-based keyword lookup keyed on identifier text. `true`/`false` are keywords, not identifiers.

- [ ] **Step 4: Implement `Lexer::tokenize`**

Hand-written scanning over `&[u8]` with an index. Skip whitespace and comments. Recognize integer literals (`[0-9]+`), float literals (`[0-9]+ '.' [0-9]+`), and string literals delimited by `"` with `\\` escapes. Every token records the byte offsets it started and ended at. On a bad character or unterminated literal or comment, push a `Diagnostic` and return `Err`. Never panic, never advance past the end of input.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p otter-lexer`
Expected: PASS.

- [ ] **Step 6: Add golden token-stream tests**

Create `tests/golden/numbers.ot`, `strings.ot`, `comments.ot`, `punct.ot`, each exercising its feature. Write a test that tokenizes each and compares against the `.expected` file, which is one `kind@start..end` pair per line. Generate the `.expected` files with `UPDATE_GOLDEN=1 cargo test -p otter-lexer`, then **read each one** and confirm it matches what you expect — this is where a silent off-by-one in spans gets caught.

Run: `cargo test -p otter-lexer` → PASS. Then `cargo test -p otter-lexer` again *without* `UPDATE_GOLDEN` to prove the comparison actually fails on mismatch (temporarily edit one `.expected` file).

- [ ] **Step 7: Commit**

```bash
git add crates/otter-lexer/
git commit -m "feat(lexer): tokens with byte spans, golden span tests"
```

---

### Task 3: AST node types

**Files:**
- Create: `crates/otter-parser/src/ast.rs`
- Create: `crates/otter-parser/src/lib.rs`

**Interfaces:**
- Consumes: `Span` from Task 0
- Produces: the AST used by every later task.
  - `Program { pub items: Vec<Item> }`
  - `Item` — `Import { path: Vec<String>, span }`, `Struct { name, fields: Vec<FieldDef>, span }`, `Enum { name, variants: Vec<VariantDef>, span }`, `Fn { name, params: Vec<Param>, ret: Option<TypeExpr>, body: Block, span }`, `Extern { abi: Option<String>, link: Option<String>, decls: Vec<ExternDecl>, span }`
    - `Extern` is declared now, in M0–M2, even though Task 4 **rejects** `extern` blocks with a diagnostic ("extern blocks arrive in M5"). Task 2 already lexes the `extern` and `unsafe` keywords, so without the variant M5 would have to change a frozen AST that four tasks code against.
    - `abi` and `link` are separate fields because M5's syntax is `extern "C" link "m" { ... }` — an ABI name and a link directive on the same block. An earlier ruling used a single `link` field for both, which cannot hold them; they were separated while the contract was still soft, which is the only moment it is cheap.
  - `ExternDecl { pub name: String, pub params: Vec<Param>, pub ret: Option<TypeExpr>, pub span: Span }`
- `Param { pub name: String, pub ty: TypeExpr, pub span: Span }`
  - **Recursive `Expr` variants are boxed.** `Binary.lhs`, `Binary.rhs`, `Unary.operand`, `FieldAccess.base`, `Cast.expr`, `Try.expr`, and `TypeExpr`'s nested types are `Box<Expr>` / `Box<TypeExpr>`. Without boxing the enum is infinitely sized and will not compile. Every later task writes `Box::new` at construction and derefs with `*` in sema and codegen.
  - `FieldDef { pub name: String, pub ty: TypeExpr, pub span: Span }` — struct fields are always named.
  - `VariantDef { pub name: String, pub fields: Vec<VariantField>, pub span: Span }`
  - `VariantField` — `Positional { pub ty: TypeExpr, pub span: Span }` for `Circle(f64)`, `Named { pub name: String, pub ty: TypeExpr, pub span: Span }` for `Rect { w: f64, h: f64 }`. A unit variant like `Red` has none.
    - **No sentinel encodes positional-ness.** An earlier contract reused `FieldDef` and left `name` empty for tuple variants, which makes `String::is_empty` mean "positional" for the whole compiler — a convention that turns a missing field name into a silently wrong type check in Tasks 8 and 9. A two-variant enum makes the distinction unrepresentable-as-a-mistake.
  - `TypeExpr` — `Name(String, Span)`, `Option(Box<TypeExpr>, Span)`, `Result(Box<TypeExpr>, Box<TypeExpr>, Span)`, `Tuple(Vec<TypeExpr>, Span)`, `Unit(Span)`
    - `()` must parse to exactly one node, and it is **`Unit`**. `Tuple` is never produced empty. Two spellings of one type would make `PartialEq` and the AST golden dumps drift, and every later task would have to special-case one of them.
  - `Block { pub stmts: Vec<Stmt>, pub span: Span }`
  - `Stmt` — `Expr(Expr, Span)`, `Const { name, ty: Option<TypeExpr>, init: Expr, span }`, `Var { name, ty: Option<TypeExpr>, init: Expr, span }`, `Return(Option<Expr>, Span)`, `If { init: Option<Box<Stmt>>, cond: Expr, then_block: Block, else_block: Option<Block>, span }`, `ForRange { name, start, end, body, span }`, `ForC { init: Option<Box<Stmt>>, cond: Option<Expr>, update: Option<Expr>, body, span }`, `Switch { subject: Expr, arms: Vec<SwitchArm>, span }`, `Break(Span)`, `Continue(Span)`, `Defer { body: Block, span }`
    - `If.init` exists because `if d := f(); d > 5.0` needs a place to put the binding that is scoped to the `if`. It holds a `Stmt::Const`; the condition refers to the name as an ordinary `Expr::Ident`. The alternative — overloading the condition's left side — makes the binding invisible in the tree, which Tasks 5, 8, and 12 then have to rediscover.
    - `ForC.init` is `Option<Box<Stmt>>` because the init clause is a complete statement (it may be a three-clause `for` variable declaration).
  - `SwitchArm { pub pattern: Pattern, pub body: Expr, pub is_default: bool, pub span: Span }`
  - `Pattern` — `Wildcard(Span)`, `UnitVariant { path: Vec<String>, span }`, `TupleVariant { path: Vec<String>, args: Vec<Pattern>, span }`, `StructVariant { path: Vec<String>, fields: Vec<(String, Pattern, Span)>, span }`, `Binding { name: String, span }`, `Literal(Literal, Span)`
  - `Literal` — `I64(i64)`, `F64(f64)`, `Bool(bool)`, `Str(StringLit)`
  - `Expr` — `Literal(Literal, Span)`, `Ident(Vec<String>, Span)`, `Binary { op: BinOp, lhs, rhs, span }`, `Unary { op: UnOp, operand, span }`, `Move(Box<Expr>, Span)`, `Call { callee: Vec<String>, args: Vec<Expr>, unsafe_call: bool, span }`, `StructLit { path: Vec<String>, fields: Vec<(String, Expr, Span)>, span }`, `FieldAccess { base, field: String, span }`, `Cast { expr, ty: TypeExpr, span }`, `Try { expr, span }`, `SwitchExpr { subject: Expr, arms: Vec<SwitchArm>, span }`
    - `Move(Box<Expr>, Span)` is a unary prefix form (`move x`). It is a distinct variant rather than a `UnOp` because its operand is an expression but its meaning is a linear-ownership transfer, not a value transformation — M3's linearity checker and codegen treat it specially. It is declared now (pre-M3) so M3 does not have to change a frozen contract four tasks code against.
    - `Call.unsafe_call` exists because spec §8.1 requires explicit `unsafe` **at the call site** to hand a raw pointer to C, in addition to the `unsafe` every declaration carries. Booleans named `unsafe*` rather than a separate node type, because the call tree is otherwise identical and a wrapper node would add a variant every later task must match through.
  - `BinOp` — `Add Sub Mul Div Rem Eq Ne Lt Le Gt Ge And Or`, with `BinOp::symbol(&self) -> &'static str`
  - `UnOp` — `Neg Not`, with `UnOp::symbol(&self) -> &'static str`
  - Every node type derives `Clone`, `Debug`, `PartialEq`

**Note:** the AST is untyped. `otter-sema` consumes it and produces HIR. Do not add type fields here.

- [ ] **Step 1: Write the failing test**

In `crates/otter-parser/tests/parse.rs`:

```rust
#[test]
fn ast_nodes_round_trip_through_debug() {
    // Construct a Program containing one of each Item and Stmt variant,
    // format with {:?}, assert the output contains the variant names.
    // This fails to compile until the types exist, which is the point.
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p otter-parser`
Expected: FAIL — `ast` module not found.

- [ ] **Step 3: Define the AST**

Write `ast.rs` with exactly the types above. `StringLit` is re-exported from `otter-lexer`. Keep every variant's `span`. Do not implement any behaviour beyond `symbol()` and `Debug`.

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p otter-parser`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/otter-parser/src/ast.rs crates/otter-parser/src/lib.rs crates/otter-parser/tests/parse.rs
git commit -m "feat(parser): AST node types with spans"
```

---

### Task 4: Parse items — imports, structs, enums, functions

**Files:**
- Create: `crates/otter-parser/src/items.rs`
- Create: `crates/otter-parser/src/error.rs`

**Interfaces:**
- Consumes: `Token`/`TokenKind` from Task 2, AST from Task 3
- Produces:
  - `Parser::new(tokens: Vec<Token>, source: &SourceMap, file: FileId) -> Parser` — the map is **borrowed**, not taken, because the caller needs it to render the diagnostics the parser produces
  - `Parser::parse_program(&mut self) -> Result<Program, Vec<Diagnostic>>`
  - `Parser::peek(&self) -> TokenKind`, `Parser::check(&self, k: TokenKind) -> bool`, `Parser::eat(&mut self, k: TokenKind) -> bool`, `Parser::expect(&mut self, k: TokenKind) -> Result<Token, ParseError>`, `Parser::error(&self, span: Span, msg: String) -> ParseError`
  - `ParseError { pub span: Span, pub message: String }`

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn parses_import_struct_enum_and_fn() {
    // Source: import std.io; struct P { x: f64, y: f64 }
    //         enum C { Red, Green } fn d(a: P, b: P) -> f64 { return 0.0; }
    // Assert item kinds, names, field names and types.
}

#[test]
fn parses_dotted_import_path() {
    // "import crypto.sha256;" -> Import { path: ["crypto", "sha256"] }
}

#[test]
fn unclosed_brace_in_fn_body_is_an_error_at_eof() {
    // "fn f() {" -> Err with message containing "unclosed" and a span at EOF.
    // This is Review Focus item 3.
}

#[test]
fn unterminated_import_path_is_an_error_not_a_panic() {
    // "import ;" -> Err.
}

#[test]
fn struct_requires_at_least_one_named_field() {
    // "struct S { }" -> Err naming the struct.
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p otter-parser`
Expected: FAIL — `Parser` does not exist.

- [ ] **Step 3: Implement the cursor primitives and item parsing**

`Parser` holds the token vector plus an index. `expect` on the wrong kind returns a `ParseError` whose message names the expected and found spellings via `describe()`. Unclosed-delimiter tracking: push the opening brace's span when entering a block, pop on close; at `Eof` with a non-empty stack, report "unclosed `{`" pointing at the matching opener with a note that EOF was reached. `parse_program` loops until `Eof`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p otter-parser`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/otter-parser/
git commit -m "feat(parser): item parsing with unclosed-delimiter diagnostics"
```

---

### Task 5: Parse statements

**Files:**
- Create: `crates/otter-parser/src/stmts.rs`
- Test: `crates/otter-parser/tests/parse.rs`

**Interfaces:**
- Consumes: `Parser` primitives from Task 4, AST from Task 3
- Produces: `Parser::parse_block(&mut self) -> Result<Block, ParseError>`, `Parser::parse_stmt(&mut self) -> Result<Stmt, ParseError>`
- `Stmt` forms produced, per spec §10.2: `Expr`, `Const`, `Var`, `Return`, `If`, `ForRange`, `ForC`, `Switch`, `Break`, `Continue`, `Defer`. `defer` takes a braced block.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn parses_bare_expression_statement() { /* "f(); ;" */ }

#[test]
fn parses_const_and_var_with_optional_type() {
    // "const x = 1;" and "const y: i64 = 2;" and "var z: f64 = 3.0;"
    // Assert ty is None for x and Some(Name("i64")) for y.
}

#[test]
fn parses_if_else_and_if_with_binding() {
    // "if d := f(); d > 5.0 { } else { }" -> If whose `init` is
    // Some(Box<Stmt::Const { name: "d", .. })) and whose `cond` is a
    // Binary whose lhs is an Expr::Ident reading "d".
    // Assert the binding name is "d".
}

#[test]
fn parses_both_for_forms() {
    // "for i in 0..10 { }" -> ForRange. "for i := 0; i < 10; i = i + 1 { }" -> ForC.
}

#[test]
fn range_operator_requires_an_upper_bound() {
    // "for i in 10 { }" -> Err mentioning the ".." operator.
}

#[test]
fn parses_switch_with_case_default_and_break() {
    // arms with tuple, struct, unit-variant and wildcard patterns.
}

#[test]
fn parses_defer_block() { /* "defer { f(); }" -> Defer */ }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p otter-parser`
Expected: FAIL — `parse_stmt` does not exist. (Task 4 already provides `parse_block`, so the original red-state gate "`parse_block` does not exist" can no longer fire. Task 4 also left `the_block_seam_consumes_the_body_and_reports_its_span` asserting an empty `stmts`, which this task must replace with real statement assertions.)

- [ ] **Step 3: Implement statement parsing**

`parse_block` consumes `{`, loops calling `parse_stmt` until `}`, and records the block span from the brace to the close. `parse_stmt` dispatches on the first token. `if` parses the condition, then an optional `{ ident :=` binding prefix that becomes a `Const` statement as the condition's left side, then `else`. Range `for` requires `in` and a range expression `a..b`. `defer` requires a braced block.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p otter-parser`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/otter-parser/
git commit -m "feat(parser): statement parsing"
```

---

### Task 6: Parse expressions

**Files:**
- Create: `crates/otter-parser/src/exprs.rs`
- Test: `crates/otter-parser/tests/parse.rs`

**Interfaces:**
- Consumes: `Parser` primitives from Task 4, AST from Task 3
- Produces: `Parser::parse_expr(&mut self) -> Result<Expr, ParseError>`. `parse_type_expr` is **already implemented by Task 4** — item parsing needs it to read a struct field or parameter type — so this task inherits it rather than writing a second one; read it first and do not duplicate it.
- Precedence, lowest to highest: `Or` → `And` → equality (`Eq Ne`) → comparison (`Lt Le Gt Ge`) → additive (`Add Sub`) → multiplicative (`Mul Div Rem`) → unary (`Neg Not`) → postfix (call, field access, `?`) → primary (literal, ident, struct literal, parens, `switch` expression)

**Note:** `&&` and `||` are lexed as single tokens in Task 2; map them to `BinOp::And` / `BinOp::Or`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn multiplication_binds_tighter_than_addition() {
    // "1 + 2 * 3" -> Binary(Add, 1, Binary(Mul, 2, 3)). Assert the tree shape.
}

#[test]
fn comparison_binds_looser_than_arithmetic() {
    // "1 + 2 < 3 * 4" -> Binary(Lt, Binary(Add,1,2), Binary(Mul,3,4)).
}

#[test]
fn unary_binds_tighter_than_multiplication() {
    // "-2 * 3" -> Binary(Mul, Unary(Neg, 2), 3). NOT Unary(Neg, Binary(...)).
}

#[test]
fn postfix_chains_left_to_right() {
    // "a.b(1).c" -> FieldAccess(Call(FieldAccess(a,b),[1]), c).
}

#[test]
fn postfix_try_and_as_associate_tightest() {
    // "f()?.x as f64" -> Cast(FieldAccess(Try(Call f), x), f64).
}

#[test]
fn parses_typed_parentheses_in_type_position() {
    // "fn g() -> Result[i64, string] { }" -> TypeExpr::Result(Name("i64"), Name("string")).
}

#[test]
fn parses_switch_expression_in_value_position() {
    // "const c = switch x { case 1 { 10 } default { 0 } };" -> Expr::SwitchExpr.
}

#[test]
fn unbalanced_parenthesis_is_an_error_not_a_panic() { /* "(1 + 2" */ }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p otter-parser`
Expected: FAIL — `parse_expr` does not exist.

- [ ] **Step 3: Implement precedence climbing**

One function per precedence level, each calling the next-tighter level, with `parse_primary` handling literals, identifiers (including dotted paths), struct literals, parenthesised expressions, and `switch` expressions. Postfix loops apply `?`, calls, and `.field` while the next token allows. `parse_type_expr` handles `Name` with dotted paths, `Option[T]`, `Result[T, E]`, `Tuple`, and bare `()`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p otter-parser`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/otter-parser/
git commit -m "feat(parser): expression parsing with precedence climbing"
```

---

### Task 7: Exhaustive match checking

**Files:**
- Create: `crates/otter-sema/src/exhaustive.rs`
- Create: `crates/otter-sema/src/lib.rs` (stub `Type`/`HIR` re-exports until Task 8)
- Create: `crates/otter-sema/tests/exhaustive.rs`

**Interfaces:**
- Consumes: `Pattern`, `VariantDef` from Task 3
- Produces:
  - `pub fn missing_cases(enum_name: &str, variants: &[VariantDef], arms: &[Pattern], has_default: bool) -> Vec<String>`
  - Returns the display names of variants not covered; empty means exhaustive.
  - `pub fn is_exhaustive(variants: &[VariantDef], arms: &[Pattern], has_default: bool) -> bool` — thin wrapper over the above

**Note:** this runs on patterns only and needs no type information. Implement it in `otter-sema` because it is a static-check concern, and unit-test it without the rest of sema present.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn wildcard_covers_everything() {
    // enum {Red,Green,Blue}, arms=[Wildcard] -> is_exhaustive true.
}

#[test]
fn all_variants_covered_needs_no_wildcard() {
    // arms=[UnitVariant Red, UnitVariant Green, UnitVariant Blue] -> exhaustive.
}

#[test]
fn reports_exactly_the_missing_variants() {
    // arms=[UnitVariant Red] -> missing == ["Green", "Blue"], in declaration order.
}

#[test]
fn binding_a_variant_covers_all_its_fields() {
    // arms=[TupleVariant(Color, [Wildcard, Wildcard])] covers Color.
}

#[test]
fn partial_field_patterns_do_not_cover() {
    // arms=[StructVariant(Rect, [("w", Wildcard)])] -> Rect still missing.
}

#[test]
fn literal_patterns_do_not_count_as_coverage() {
    // A non-enum switch over i64 is never exhaustive without a default.
    // assert missing_cases on a non-enum variant list reports incompleteness.
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p otter-sema`
Expected: FAIL — `missing_cases` does not exist.

- [ ] **Step 3: Implement `missing_cases`**

A wildcard arm or a `default` makes everything exhaustive. Otherwise, start from the full variant list and remove variants matched by a `UnitVariant`, `TupleVariant`, or `StructVariant` arm. Compare variant paths by their final segment, since patterns carry the same qualified path the enum declares. Return the survivors in declaration order. This runs before type checking, so it must not depend on resolved types.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p otter-sema`
Expected: PASS — 7 tests.

- [ ] **Step 5: Commit**

```bash
git add crates/otter-sema/
git commit -m "feat(sema): exhaustiveness checking for enum switches"
```

---

### Task 8: Type checker core — types, scopes, inference

**Files:**
- Create: `crates/otter-sema/src/ty.rs`
- Create: `crates/otter-sema/src/hir.rs`
- Create: `crates/otter-sema/src/scope.rs`
- Create: `crates/otter-sema/src/check_expr.rs`
- Create: `crates/otter-sema/src/error.rs`
- Create: `crates/otter-sema/tests/check.rs`

**Interfaces:**
- Consumes: AST from Task 3, `Span`/`Diagnostic` from Tasks 0–1
- Produces:
  - `TypeKind` — `I64`, `F64`, `Bool`, `Str`, `Unit`, `Struct(String)`, `Enum(String)`, `Option(Box<Type>)`, `Result(Box<Type>, Box<Type>)`, `Func(Vec<Type>, Box<Type>)`, `Never`
  - `pub struct Type { pub kind: TypeKind, pub span: Span }` with `PartialEq` ignoring spans, `eq_ignoring_span`, and `Display`
  - `Mutability { Const, Var }`
  - HIR is a **fully parallel tree with distinct names**, so it can never be confused with the AST:
    - `HirProgram { pub items: Vec<HirItem> }`
    - `HirItem` — `HirStruct { name, fields, span }`, `HirEnum { name, variants, span }`, `HirFn { name, params, ret, body, span }`, `HirImport { path, span }`
    - `HirFn { pub name: String, pub params: Vec<HirParam>, pub ret: Type, pub body: HirBlock, pub span: Span }`
    - `HirParam { pub name: String, pub ty: Type, pub span: Span }`
    - `HirBlock { pub stmts: Vec<HirStmt>, pub span: Span }`
    - `HirStmt` — same variants as `Stmt`, with `ty: Type` on expression statements and `mutability: Mutability` on `HirStmt::Const` / `HirStmt::Var`
    - `HirExpr` — same variants as `Expr`, each carrying an extra `ty: Type` field
    - `HirSwitchArm { pub pattern: HirPattern, pub body: HirExpr, pub is_default: bool, pub span: Span }`
    - `HirPattern` — same variants as `Pattern`, plus the bound type and mutability on `Binding`
    - All derive `Clone`, `Debug`, `PartialEq`
  - `pub struct TypeError { pub span: Span, pub message: String }` with `into_diagnostic(&self, file: FileId) -> Diagnostic`
  - `pub fn check_program(ast: &Program, sm: &SourceMap, file: FileId) -> Result<HirProgram, Vec<Diagnostic>>`
  - Builtins are predeclared in the root scope: `print_str(Str)`, `print_i64(I64)`, `print_f64(F64)`. Otter-facing names are exactly these three.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn infers_i64_from_integer_literal() { /* const x = 1; -> I64 */ }

#[test]
fn infers_f64_from_float_literal() { /* const x = 1.5; -> F64 */ }

#[test]
fn function_parameters_and_returns_are_checked() {
    // fn f(a: i64) -> i64 { return a; } type-checks.
}

#[test]
fn wrong_return_type_is_a_diagnostic_with_a_span() {
    // fn f() -> i64 { return 1.0; } -> Err, diagnostic span on the literal.
}

#[test]
fn duplicate_binding_in_one_scope_is_an_error_not_last_wins() {
    // "fn f() { const x = 1; const x = 2; }" -> Err naming the second x.
    // This is Review Focus item 4.
}

#[test]
fn two_functions_named_main_is_an_error() {
    // Two `fn main` items -> Err. Related: a program with no main is Task 10.
}

#[test]
fn no_implicit_narrowing_conversion() {
    // fn f(a: i64) -> f64 { return a; } -> Err naming f64/i64.
}

#[test]
fn explicit_as_cast_permits_an_intentional_conversion() {
    // fn f(a: i64) -> f64 { return a as f64; } type-checks.
}

#[test]
fn as_cast_to_an_unrelated_type_is_an_error() {
    // "return 1 as bool;" -> Err naming the permitted cast pairs.
}

#[test]
fn as_cast_rejects_conversion_to_struct() {
    // "return 1 as Point;" -> Err: casts are only between primitives at M0-M2.
}

#[test]
fn struct_literal_fields_are_checked_by_name() {
    // Point { x: 1, y: 2 } -> Err: field y expects f64, got i64.
}

#[test]
fn unknown_field_in_struct_literal_is_an_error() { /* Point { z: 1.0 } */ }

#[test]
fn if_condition_must_be_bool() { /* if 1 { } -> Err */ }

#[test]
fn builtin_calls_resolve() {
    // print_i64(1) type-checks; print_str(1) does not.
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p otter-sema`
Expected: FAIL — `check_program` does not exist.

- [ ] **Step 3: Implement `Type` and `TypeKind`**

`Display` writes `i64`, `f64`, `bool`, `string`, `()`, `Option[T]`, `Result[T, E]`, and the bare struct/enum name. `PartialEq` derives on `TypeKind` only, so a `Type` with a different span still compares equal.

- [ ] **Step 4: Implement scopes and duplicate detection**

`Scope` holds a `Vec<HashMap<String, Binding>>` stack. `Binding { ty: Type, mutability: Mutability, span: Span }`. `declare` inserts into the current scope and returns an error if the name is already present **in that scope** — shadowing an outer scope is allowed. Item names (`fn`, `struct`, `enum`) go into the root scope, so duplicates there are errors too.

- [ ] **Step 5: Implement `check_program` and `check_expr`**

Two passes. First, collect all item signatures (struct shapes, enum variants, function signatures) into the root scope — this permits mutual recursion. Second, check each function body: push a scope, declare parameters, infer each statement's type bottom-up, and unify against annotations where present. Literal widening: an integer literal unifies with `i64`, a float literal with `f64`, and a literal assigned to a wider annotated integer type (M4's concern) is **not** implemented yet. `if` conditions must be `bool`. Calls resolve builtins and user functions by name; a call to an unknown name is an error naming the name.

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p otter-sema`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/otter-sema/
git commit -m "feat(sema): types, scopes, duplicate detection, expression inference"
```

---

### Task 9: Pattern checking — `Option`, `Result`, and switch typing

**Files:**
- Create: `crates/otter-sema/src/check_pat.rs`
- Modify: `crates/otter-sema/src/check_expr.rs` (switch arms)
- Modify: `crates/otter-sema/tests/check.rs`

**Interfaces:**
- Consumes: `check_expr` internals from Task 8, `missing_cases` from Task 7, AST patterns from Task 3
- Produces:
  - `pub fn check_pattern(pat: &HirPattern, scrutinee: &Type, out_bindings: &mut Vec<(String, Type, Mutability, Span)>, variants: &dyn Fn(&str) -> Option<Vec<VariantDef>>) -> Result<(), TypeError>`
  - `TypeError` is declared in Task 8; this task populates it, it does not redefine it
  - `Option<T>` is a struct named `Option` with variants `None` (no fields) and `Some` (one field of type `T`). `Result<T, E>` is a struct named `Result` with variants `Ok` (one field of `T`) and `Err` (one field of `E`).
  - Every `switch` arm body is checked against a single expected result type, unified across arms.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn matches_some_and_binds_the_payload() {
    // fn f(o: Option[i64]) -> i64 { switch o { case None { 0 } case Some(x) { x } } }
}

#[test]
fn matching_none_against_some_payload_is_an_error() {
    // case Some(x) against Option[i64] where x then used as bool -> Err.
}

#[test]
fn result_ok_and_err_bind_different_payload_types() {
    // fn f(r: Result[i64, string]) -> i64 {
    //     switch r { case Ok(v) { v } case Err(e) { 0 } } }
}

#[test]
fn try_on_a_result_returns_the_ok_value() {
    // fn f() -> Result[i64, string] { let v = g()?; return Ok(v); }
    // Assert the enclosing return type propagates the Err type.
}

#[test]
fn try_on_a_non_result_is_an_error() { /* f()?.x where f returns i64 */ }

#[test]
fn non_exhaustive_switch_on_enum_is_an_error_listing_missing_cases() {
    // Err message contains "not exhaustive" and both missing variant names.
}

#[test]
fn all_arms_must_yield_the_same_type() {
    // arms yielding i64 and f64 -> Err naming the mismatching arm.
}

#[test]
fn switch_on_a_non_enum_without_default_is_an_error() {
    // switch over i64 with only literal arms -> Err suggesting a default arm.
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p otter-sema`
Expected: FAIL — `check_pattern` does not exist.

- [ ] **Step 3: Register `Option` and `Result` as builtins**

Add `Option<T>` and `Result<T, E]` to the root scope as struct-like enums with the variant shapes in Interfaces. Give them a reserved `Span { start: 0, end: 0 }` so diagnostics about them never point at user code.

- [ ] **Step 4: Implement `check_pattern` and wire switch checking**

`check_pattern` walks the pattern against the scrutinee type. `Binding` declares a new binding with the binding's `var`-like mutability (patterns bind by value). `TupleVariant` looks up the variant's field types and checks each sub-pattern positionally, reporting "expected N fields, got M" on mismatch. `StructVariant` matches by field name and requires all fields. `Literal` unifies the literal type with the scrutinee type. Switch checking: check the subject, then each arm's pattern against it, then unify all arm bodies against the first arm's type.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p otter-sema`
Expected: PASS.

- [ ] **Step 6: Add golden HIR dumps**

Create `tests/golden/option.ot` and `tests/golden/result.ot` using the programs from Task 13. Add a test that checks each and compares `{:#?}` of the resulting `HirProgram` against `tests/golden/<name>.expected`. Generate with `UPDATE_GOLDEN=1 cargo test -p otter-sema`, then **read both dumps and confirm the inferred types are right** — this is the cheapest way to catch an inference bug that still type-checks.

- [ ] **Step 7: Commit**

```bash
git add crates/otter-sema/
git commit -m "feat(sema): pattern checking for Option, Result and switch arms"
```

---

### Task 10: Literal emission and `main` discovery

**Files:**
- Create: `crates/otter-codegen/src/module.rs`
- Create: `crates/otter-codegen/src/ir.rs`
- Create: `crates/otter-codegen/src/lib.rs`
- Create: `crates/otter-codegen/tests/codegen.rs`

**Interfaces:**
- Consumes: `Type`/`TypeKind` and `HirProgram` from Tasks 8–9
- Produces:
  - `pub struct Module { lines: Vec<String>, strings: Vec<(String, String)>, next_value: u32 }`
  - `Module::new() -> Module`
  - `Module::push(&mut self, line: impl Into<String>)`
  - `Module::intern_string(&mut self, bytes: &[u8]) -> String` — returns a global name like `@.str.0`, recording `@.str.0 = private unnamed_addr constant [N x i8] c"..."`
  - `Module::next_value(&mut self) -> String` — returns `%t0`, `%t1`, ...
  - `Module::finish(&self) -> String` — emits the header below, then type aliases, then string globals, then the accumulated body
  - `pub fn llvm_type_string(t: &TypeKind) -> String` — `i64`, `double`, `i1`, `{ i32, i32, i8* }` for `Str`, `void`
  - `pub fn find_main(items: &[HirItem]) -> Option<&HirFn>`
  - `pub fn check_main_exists(items: &[HirItem]) -> Result<(), Diagnostic>`

  The module header is fixed by LLVM 17 opaque pointers and must be emitted verbatim:

```llvm
; Generated by the Otter compiler. Do not edit.
source_filename = "main.ot"
target datalayout = "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-f80:128-n8:16:32:64-S128"
target triple = "x86_64-pc-windows-msvc"

%otter_str = type { i32, i32, i8* }

declare void @otter_print_str(i8*)
declare void @otter_print_i64(i64)
declare void @otter_print_f64(double)
```

**All three print functions return `void`.** An earlier draft of this plan declared
`i32 @otter_print_str(i8*)`; the master plan's ledger L1 is authoritative and says
`void`. Codegen must emit `void`.

`target triple` is `"x86_64-pc-windows-msvc"` on Windows and `"x86_64-unknown-linux-gnu"` otherwise, chosen from `cfg!(target_os)`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn finish_emits_the_opaque_pointer_header() {
    // Assert the output contains "target datalayout" and "%otter_str = type".
}

#[test]
fn intern_string_dedupes_identical_bytes() {
    // intern "hi" twice -> both return "@.str.0" and only one global is emitted.
}

#[test]
fn string_global_is_nul_terminated_and_escaped() {
    // bytes b"a\"b" -> global contains \22 and ends with \00; length is 4.
}

#[test]
fn missing_main_is_a_diagnostic_not_a_link_failure() {
    // check_main_exists on a program with no main -> Err naming the file.
    // This is Review Focus item 2.
}

#[test]
fn main_may_be_absent_or_return_i64() {
    // main with no return type passes; main with -> i64 passes; -> bool fails.
}

#[test]
fn llvm_type_mapping_matches_the_spec() {
    // i64->"i64", f64->"double", bool->"i1", Str->"{ i32, i32, i8* }", Unit->"void".
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p otter-codegen`
Expected: FAIL — `Module` does not exist.

- [ ] **Step 3: Implement `Module` and `ir::llvm_type_string`**

`next_value` hands out sequential temporaries. `intern_string` keeps a map from the byte slice to a global name so identical literals share one global. `finish` writes the header verbatim, then every interned string global, then the body. Escape non-printable and `"` `\` characters as `\XX`.

- [ ] **Step 4: Implement `find_main` and `check_main_exists`**

`find_main` returns the item named `main` whose kind is a function. `check_main_exists` returns a `Diagnostic` with `Span { start: 0, end: 0 }` when absent — sema already rejected duplicates in Task 8, so presence is the only check here.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p otter-codegen`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/otter-codegen/
git commit -m "feat(codegen): LLVM module scaffold, string globals, main discovery"
```

---

### Task 11: Expression codegen

**Files:**
- Create: `crates/otter-codegen/src/expr_gen.rs`
- Create: `crates/otter-codegen/src/patterns.rs`
- Create: `crates/otter-codegen/tests/golden/ops.ot`
- Create: `crates/otter-codegen/tests/golden/ops.expected.ll`

**Interfaces:**
- Consumes: `HirProgram`, `HirExpr`, `HirSwitchArm` from Task 8, `Module` from Task 10
- Produces:
  - `pub struct FnCtx { pub module: Module, pub ret_type: TypeKind, pub next_slot: u32 }`
  - `FnCtx::alloc_slot(&mut self) -> String` — a fresh alloca name
  - `pub fn gen_expr(ctx: &mut FnCtx, expr: &HirExpr) -> GenValue`
  - `pub struct GenValue { pub value: String, pub ty: TypeKind }` — `value` is an LLVM SSA name or a literal like `42`
  - `pub fn gen_expr_to_slot(ctx: &mut FnCtx, expr: &HirExpr) -> String` — alloca plus store, returning the slot name; the single entry point for statements, `return`, and match arms
  - `pub fn gen_match(ctx: &mut FnCtx, subject: &GenValue, arms: &[HirSwitchArm]) -> String` — returns the slot holding the arm result

**Interim arithmetic semantics (Review Focus items 1 and 5):** M0–M2 emits LLVM wrapping operations. Map `Add Sub Mul` to `add`/`sub`/`mul` with `nsw`/`nuw` **omitted** so wraparound is well-defined rather than UB; `Div Rem` to `sdiv`/`srem` with no zero check. Comparisons emit `icmp`/`fcmp`; `And`/`Or` emit `and`/`or` on `i1`. These are the exact opcodes the golden test pins, and Task 14's successor in M4 replaces them with checked calls.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn integer_add_emits_wrapping_add_without_nsw() {
    // gen "const x = 1 + 2;" -> IR contains "add i64" and NOT "add nsw".
    // This is Review Focus item 5.
}

#[test]
fn integer_divide_emits_sdiv_with_no_zero_guard() {
    // gen "const x = 1 / 0;" -> IR contains "sdiv i64".
    // Document the hazard: sdiv by zero is undefined until M4.
    // This is Review Focus item 1.
}

#[test]
fn comparisons_emit_the_matching_predicate() {
    // "1 < 2" -> icmp slt; "1.0 < 2.0" -> fcmp olt.
}

#[test]
fn boolean_operators_operate_on_i1() { /* "true && false" -> "and i1" */ }

#[test]
fn call_emits_a_call_instruction_naming_the_runtime_symbol() {
    // print_i64(1) -> "call void @otter_print_i64(i64 1)".
}

#[test]
fn match_on_option_tests_the_tag_then_loads_the_payload() {
    // switch on Option[i64] -> IR branches on an icmp against the tag field
    // and loads the Some payload into the binding slot.
}

#[test]
fn match_on_result_distinguishes_ok_and_err() {
    // Both arms tested against distinct tag values; each loads its payload.
}

#[test]
fn wildcard_arm_needs_no_tag_test() { /* last arm emits no icmp */ }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p otter-codegen`
Expected: FAIL — `FnCtx` does not exist.

- [ ] **Step 3: Implement `FnCtx` and `gen_expr`**

`alloca` every local into a distinct slot so no expression is evaluated twice. Binary operators emit the mapped instruction and `%tmp = add ...`, returning `%tmp`. `StructLit` allocs an aggregate slot and stores each field by index. `FieldAccess` loads from the aggregate's declared field index. `Cast` emits the conversion LLVM needs, with no implicit narrowing — `i64` to `double` is `sitofp`, `f64` to `i64` is `fptosi`, `bool` from a comparison is the `i1` itself.

- [ ] **Step 4: Implement `gen_match`**

Allocate a slot for the result. For each non-wildcard, non-default arm, load the scrutinee's tag field, compare it against the variant's tag constant, and `br i1` to that arm's block; the last block is the merge point carrying the result. A wildcard or default arm gets no tag test. Bindings become stores into slots for their pattern-matched field.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p otter-codegen`
Expected: PASS.

- [ ] **Step 6: Add the golden IR test for operators**

Write `tests/golden/ops.ot` covering add, subtract, multiply, divide, remainder, all six comparisons, both boolean operators, and one call per runtime builtin. Generate `ops.expected.ll` with `UPDATE_GOLDEN=1 cargo test -p otter-codegen`, then **read the generated IR and confirm every opcode matches the mapping in Interfaces**. This file is the contract M4 changes.

- [ ] **Step 7: Commit**

```bash
git add crates/otter-codegen/
git commit -m "feat(codegen): expression lowering, matching, wrapping arithmetic contract"
```

---

### Task 12: Statement and function codegen, including `defer`

**Files:**
- Create: `crates/otter-codegen/src/fn_gen.rs`
- Create: `crates/otter-codegen/src/stmt_gen.rs`
- Create: `crates/otter-codegen/src/lib.rs` (complete `emit_program`)
- Create: `crates/otter-codegen/tests/golden/control_flow.ot`
- Create: `crates/otter-codegen/tests/golden/control_flow.expected.ll`

**Interfaces:**
- Consumes: `FnCtx`/`gen_expr_to_slot`/`gen_match` from Task 11, `Module` from Task 10, `HirProgram` from Task 8
- Produces:
  - `pub fn emit_program(hir: &HirProgram, entry: &str) -> String` — validates `main` exists, emits the module, emits every function, returns the `.ll` text
  - `pub fn gen_fn(module: &mut Module, f: &HirFn) -> ()`
  - `pub fn gen_stmt(ctx: &mut FnCtx, stmt: &HirStmt, defers: &mut Vec<String>) -> Flow`
  - `pub enum Flow { Fallthrough, Break, Continue, Return }`
  - `pub fn gen_block(ctx: &mut FnCtx, block: &HirBlock, defers: &mut Vec<String>) -> Flow`
  - `entry` is a path the driver uses for `source_filename`, e.g. `main.ot`

**`defer` lowering (spec §5.5):** `defers` collects `gen_expr_to_slot` results for the deferred expressions of the current scope in registration order. On any scope exit — `Flow::Return`, `Break`, `Continue`, or normal fallthrough — emit the stored deferred expressions in reverse order before leaving. A nested `if` or `for` pushes a new `defers` vector so only its own defers run at its own exit.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn function_emits_an_entry_block_and_a_return() { /* fn f() -> i64 { return 1; } */ }

#[test]
fn if_else_emits_br_and_a_phi_or_slot() {
    // Assert exactly two conditional branches and a merge label.
}

#[test]
fn range_for_lowers_to_a_counted_loop() {
    // "for i in 0..10 { }" -> phi-free counted loop with an icmp exit test
    // comparing against 10.
}

#[test]
fn three_clause_for_lowers_to_a_counted_loop() { /* init, cond, update */ }

#[test]
fn break_and_continue_branch_to_the_loop_labels() {
    // Assert branch targets exist as labels in the emitted IR.
}

#[test]
fn defer_emits_in_reverse_order_before_return() {
    // "fn f() { print_i64(1); defer { print_i64(2); } defer { print_i64(3); } return; }"
    // -> call @otter_print_i64(3) appears before call @otter_print_i64(2).
}

#[test]
fn defer_runs_on_the_return_path_only_once() {
    // One defer, one return: exactly one deferred call in the output.
}

#[test]
fn nested_scope_defer_does_not_leak_to_the_outer_scope() {
    // A defer inside a for body must not run at the outer function's return.
}

#[test]
fn empty_function_with_unit_return_emits_ret_void() { /* fn f() { } */ }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p otter-codegen`
Expected: FAIL — `gen_stmt` does not exist.

- [ ] **Step 3: Implement `gen_fn`**

Emit `define <ret> @<name>(<params>) {`, an `entry:` label, one `alloca` per parameter plus a store from the incoming argument, then the body, then a terminator. A `-> i64` function that falls off the end returns `0`; a unit function emits `ret void`.

- [ ] **Step 4: Implement `gen_stmt` and `gen_block`**

`gen_expr` statements discard their value. `Const`/`Var` alloca and initialise, or initialise an existing slot for a loop variable at each iteration. `If` evaluates the condition, branches to then/else blocks, and merges. `ForRange` computes the length once, allocs the index, and loops while `i < len`. `ForC` emits init, then the condition test at the top of each iteration, then update at the bottom. `Switch` uses `gen_match`. `Return` emits the stored defers in reverse, then the value and `ret`.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p otter-codegen`
Expected: PASS.

- [ ] **Step 6: Add the golden IR test for control flow**

Write `tests/golden/control_flow.ot` with a function containing `if`/`else`, a range `for` with `break` and `continue`, a three-clause `for`, a `switch` over `Option[i64]`, and two `defer`s. Generate `control_flow.expected.ll` with `UPDATE_GOLDEN=1`, then read it and confirm the label structure and the reversed defer order are correct.

- [ ] **Step 7: Commit**

```bash
git add crates/otter-codegen/
git commit -m "feat(codegen): statement and function lowering with static defer"
```

---

### Task 13: Driver, CLI, and end-to-end binaries

**Files:**
- Create: `crates/otter-driver/src/main.rs`
- Create: `crates/otter-driver/src/pipeline.rs`
- Create: `crates/otter-driver/tests/e2e.rs`
- Create: `crates/otter-driver/tests/programs/{hello,fib,option,result,hanoi}.ot`

**Interfaces:**
- Consumes: everything from Tasks 0–12
- Produces:
  - `pub fn compile_to_binary(source: &str, entry_name: &str, out: &Path, tc: &Toolchain) -> Result<(), Vec<Diagnostic>>`
  - CLI: `otter build <file.ot> [-o <out>] [--emit-ir]`, plus `otter check <file.ot>`, `otter version`
  - Exit codes: `EXIT_OK` 0, `EXIT_COMPILE_ERROR` 1 (from Task 1)
  - `--emit-ir` prints the `.ll` to stdout and stops before invoking LLVM

- [ ] **Step 1: Write the failing end-to-end tests**

In `crates/otter-driver/tests/e2e.rs`, each test compiles a program to a temporary executable, runs it, and asserts stdout:

```rust
#[test]
fn hello_world_prints_and_exits_zero() {
    // programs/hello.ot -> "Hello, Otter!\n", exit code 0.
}

#[test]
fn fibonacci_loop_prints_the_tenth_number() { /* prints "55\n" */ }

#[test]
fn option_matching_selects_the_some_payload() { /* prints "42\n" */ }

#[test]
fn result_propagation_returns_the_ok_value() { /* prints "7\n" */ }

#[test]
fn recursion_and_enums_compose_in_hanoi() {
    // programs/hanoi.ot counts moves for 3 disks -> "7\n".
}

#[test]
fn a_type_error_produces_a_caret_diagnostic_and_exit_1() {
    // Bad program -> stderr contains "error:" and a caret line; exit code 1.
}

#[test]
fn emit_ir_flag_prints_llvm_without_invoking_llc() {
    // --emit-ir output contains "target datalayout".
}

#[test]
fn missing_llvm_toolchain_fails_with_an_install_hint() {
    // With OTTER_LLC=/nonexistent: exit 1, stderr mentions winget install LLVM.LLVM.
}
```

- [ ] **Step 2: Write the five test programs**

`hello.ot`:

```c
fn main() {
    print_str("Hello, Otter!");
    print_str("\n");
}
```

`fib.ot`:

```c
fn main() {
    var n = 0;
    var a = 0;
    var b = 1;
    for i in 0..10 {
        var t = a + b;
        a = b;
        b = t;
        n = a;
    }
    print_i64(n);
    print_str("\n");
}
```

`option.ot`:

```c
fn unwrap_or_zero(o: Option[i64]) -> i64 {
    switch o {
        case None { 0 }
        case Some(v) { v }
    }
}

fn main() {
    print_i64(unwrap_or_zero(Some(42)));
    print_str("\n");
}
```

`result.ot`:

```c
fn inner() -> Result[i64, string] {
    return Ok(7);
}

fn outer() -> Result[i64, string] {
    let v = inner()?;
    return Ok(v);
}

fn main() {
    switch outer() {
        case Ok(v) { print_i64(v); }
        case Err(e) { print_i64(0); }
    }
    print_str("\n");
}
```

`hanoi.ot`:

```c
fn count(d: i64) -> i64 {
    if d <= 1 { return 1; }
    return count(d - 1) + count(d - 1);
}

fn main() {
    print_i64(count(3));
    print_str("\n");
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p otter-driver`
Expected: FAIL — `compile_to_binary` does not exist.

- [ ] **Step 4: Implement `pipeline::compile_to_binary`**

In order: lex, parse, check, emit. Any `Err` returns the diagnostic vector immediately — nothing downstream runs on a broken program. On success write the `.ll` next to the output as `out.ll`, then assemble and link.

**Backend selection follows the master plan's ledger, and `llc` is NOT available on
Windows.** The Windows LLVM distribution ships `clang.exe` and `clang-cl.exe` but no
`llc.exe`. So:

- If `Toolchain.found == "llc"`: `llc -filetype=obj -o out.o out.ll`.
- If `Toolchain.found == "clang"`: `clang -x ir out.ll -c -o out.o`.
- Link: `clang out.o -o <out> -lpthread -ldl -lm` on Linux. On Windows, link the same
  way and let clang pick its default target — **do not assume MSVC**. This
  development machine has no Visual Studio and no MSVC CRT, so `link.exe` is
  unavailable and MSVC-style `link.exe /entry:mainCRTStartup` linking cannot work.
  If MSVC *is* present, `clang` drives `link.exe` automatically with no extra flags.
- On any linker failure, report a diagnostic naming the missing library or the
  missing CRT rather than dumping raw linker output.

- [ ] **Step 5: Implement the CLI**

`main` parses arguments, calls `detect()` first so a missing toolchain fails with an install hint before any compilation, then calls the pipeline. On diagnostics, render each via `SourceMap::render` to stderr and return `EXIT_COMPILE_ERROR`.

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p otter-driver`
Expected: PASS — 8 tests. If linking fails on Windows, the runtime `.lib` is not being produced; check `build.rs`.

- [ ] **Step 7: Run the whole workspace**

Run: `cargo test --workspace`
Expected: all tests pass, no warnings.

- [ ] **Step 8: Commit**

```bash
git add crates/otter-driver/
git commit -m "feat(driver): CLI, pipeline, and end-to-end native binaries"
```

---

### Task 14: Fuzzing — the compiler never panics

**Files:**
- Create: `fuzz/Cargo.toml`
- Create: `fuzz/fuzz_targets/lex_parse.rs`
- Create: `fuzz/fuzz_targets/check.rs`
- Modify: `crates/otter-lexer/src/lib.rs`, `crates/otter-parser/src/lib.rs` (make `tokenize` and `parse_program` reachable from the fuzz target)
- Create: `crates/otter-driver/tests/never_panics.rs`

**Interfaces:**
- Consumes: `Lexer::tokenize` (Task 2), `Parser::parse_program` (Task 4–6), `check_program` (Task 8)
- Produces: a `cargo-fuzz` package with two targets, and a deterministic corpus-driven regression test

- [ ] **Step 1: Write the deterministic no-panic test first**

In `crates/otter-driver/tests/never_panics.rs`, a test that feeds a table of ~40 deliberately malformed inputs — truncated tokens, stray Unicode, unbalanced delimiters, NUL bytes, deeply nested brackets, a 100k-character identifier, unterminated block comments in every position — and asserts that each either succeeds or returns diagnostics, never panicking. Deep-nesting inputs must produce a diagnostic rather than a Rust stack overflow.

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p otter-driver --test never_panics`
Expected: FAIL — at least one input panics or overflows the stack, which is the point of the task.

- [ ] **Step 3: Add a nesting depth limit to the parser**

Track recursion depth in `Parser`; on exceeding 256 nested blocks, parentheses, or calls, return a `ParseError` with "nesting too deep" instead of recursing. This is the fix for the stack overflow and it applies to `items.rs`, `stmts.rs`, and `exprs.rs`.

- [ ] **Step 4: Re-run until the test passes**

Run: `cargo test -p otter-driver --test never_panics`
Expected: PASS.

- [ ] **Step 5: Write the fuzz targets**

`fuzz_targets/lex_parse.rs` takes arbitrary bytes, converts lossily to a `String`, and calls `tokenize` then, on success, `parse_program`. `fuzz_targets/check.rs` runs `check_program` on the parsed AST. Both discard results; they exist to find panics. The `Cargo.toml` declares `cargo-fuzz = true` and depends on the three crates plus `libfuzzer-sys`.

- [ ] **Step 6: Run the fuzzers briefly**

Run: `cargo +nightly fuzz run lex_parse -- -max_total_time=120`
Then: `cargo +nightly fuzz run check -- -max_total_time=120`
Expected: no crashes. On any crash, fix the panic and re-run.

- [ ] **Step 7: Commit the corpus**

```bash
git add fuzz/ crates/otter-driver/tests/never_panics.rs crates/otter-lexer/src/lib.rs crates/otter-parser/src/
git commit -m "test: fuzz targets and parser nesting limit; compiler never panics on input"
```

---

## Definition of Done for M0–M2

- `cargo test --workspace` passes with no warnings.
- `otter build tests/programs/hello.ot -o hello.exe` produces a native binary that prints `Hello, Otter!` on both Windows and Linux.
- `option.ot`, `result.ot`, `fib.ot`, and `hanoi.ot` each compile and produce their expected output.
- Every diagnostic in the test suite renders with a caret at the right column.
- 120 seconds of fuzzing per target finds no panic.
- Spec §10.2's "Not yet available" table is accurate — no slice, map, `alloc`, `move`, `extern`, or string-operation feature slipped in early.

## What the next plan covers

M3: `alloc`/`dealloc`, `defer` extended to unwind paths, arenas, refcounted strings with `rc` in the string header, `move` linearity checking, and string operations and interpolation — at which point Task 11's wrapping-arithmetic contract is replaced by checked arithmetic from M4.