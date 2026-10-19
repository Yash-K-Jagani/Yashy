# M11 Implementation Plan — Adoption: formatter, test runner, LSP, tutorial, installer

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give other people what they need to actually use Otter without the author present: an opinionated formatter, a built-in test runner, a language server, syntax highlighting, a tutorial that works standalone, and a one-command installer. M11 turns Otter from "a compiler that works" into "a programming language people can adopt."

**Architecture:** All tooling is Rust code in `crates/otter-driver` plus a documentation tree under `docs/tutorial/`. The formatter and language server consume the compiler's own AST (`otter-parser` and `otter-sema`) so formatting and diagnostics share one source of truth. The installer is a shell script plus prebuilt artifacts published to releases.

**Tech Stack:** Rust 1.99, `tower-lsp` for the language server, `syn`/`serde` for parsing, `tree-sitter` or `syntect` for highlighting (whichever builds on stable without C deps), shell script for the installer.

**Spec:** §8 (tooling), §9.2 (diagnostics), §8.3 (formatter contract), §8.4 (tutorial contract). **Master plan:** `2026-10-16-otter-master-plan.md` — read its Interface Ledger and Global Constraints first.

## Prerequisites

- M0–M8 complete, CI green on Windows and Linux.
- The compiler's AST is stable enough that formatting rules do not change weekly.

## Scope

**In:** `otter fmt`, `otter test`, `otter run`, `otter init`, `otter doc`, a language server, syntax highlighting, a 30-minute tutorial, a one-command installer, an error-message quality pass.

**Out, deliberately:** a package manager, a package registry, IDE debugging integration, a REPL, async/await, a GC. Those belong to M12+.

## Global Constraints

- **No new runtime dependency.** Tooling is Rust; it must not link LLVM or require clang at tool time.
- **The formatter never changes program semantics.** It only reprints whitespace and punctuation that the grammar already makes unambiguous.
- **Every diagnostic still carries a span and renders with a caret.** Tooling must not regress the diagnostic contract from M0–M4.
- **`otter test` runs the same pipeline as `otter build`.** A program that fails to type-check fails the test suite with the same diagnostic it would fail compilation with.

---

### Task 1: `otter fmt` — opinionated formatter

**Files:**
- Create: `crates/otter-driver/src/fmt.rs`
- Create: `crates/otter-driver/tests/fmt.rs`
- Modify: `crates/otter-driver/src/main.rs` (add `fmt` subcommand)
- Modify: `crates/otter-driver/src/lib.rs` (re-export)

**Interfaces:**
- Consumes: parsed AST from `otter-parser`
- Produces: reformatted source text, one canonical style

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn formatting_is_idempotent() {
    // Format twice, output is identical.
}

#[test]
fn formatting_preserves_semantics() {
    // Formatted source compiles to identical IR.
}

#[test]
fn trailing_semicolon_in_switch_arm_is_kept() {
    // `case Some(v) { print_i64(v); }` stays as written.
}
```

- [ ] **Step 2: Run to verify they fail** → FAIL.

- [ ] **Step 3: Implement**

One pass over the AST that re-emits source with:
- 4-space indentation, no tabs.
- One statement per line.
- Braces on the same line as the keyword (`fn f() {`), closing brace on its own line.
- No trailing whitespace, single trailing newline.

- [ ] **Step 4: Run to verify they pass** → PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/otter-driver/src/fmt.rs crates/otter-driver/tests/fmt.rs crates/otter-driver/src/main.rs crates/otter-driver/src/lib.rs
git commit -m "feat(tooling): otter fmt, an opinionated formatter"
```

---

### Task 2: `otter test` — built-in test runner

**Files:**
- Create: `crates/otter-driver/src/test_runner.rs`
- Create: `crates/otter-driver/tests/test_runner.rs`
- Modify: `crates/otter-driver/src/main.rs` (add `test` subcommand)

**Interfaces:**
- Consumes: source files in a directory, `#[test]` attribute on functions
- Produces: pass/fail summary, exit code 0 or 1

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn a_test_function_with_no_args_runs_and_reports_pass() {}
#[test]
fn a_test_that_panics_is_reported_as_fail_with_position() {}
#[test]
fn test_discovery_skips_non_test_functions() {}
```

- [ ] **Step 2: Run to verify they fail** → FAIL.

- [ ] **Step 3: Implement**

Scan `src/` for functions annotated `#[test]`, compile them into a temporary test binary with a generated `main` that calls each test function and reports results, run it, print pass/fail count.

- [ ] **Step 4: Run to verify they pass** → PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/otter-driver/src/test_runner.rs crates/otter-driver/tests/test_runner.rs crates/otter-driver/src/main.rs
git commit -m "feat(tooling): otter test, built-in test runner"
```

---

### Task 3: `otter run` — compile and run in one step

**Files:**
- Modify: `crates/otter-driver/src/main.rs` (add `run` subcommand)

**Interfaces:**
- Consumes: a source file path
- Produces: compiles to a temp directory, runs the executable, forwards its exit code and output

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn otter_run_compiles_and_executes_a_source_file() {}
#[test]
fn otter_run_forwards_the_program_exit_code() {}
#[test]
fn otter_run_prints_program_stdout_and_stderr() {}
```

- [ ] **Step 2: Run to verify they fail** → FAIL.

- [ ] **Step 3: Implement**

`otter run file.ot` compiles to a temporary output path (next to the source or in the system temp dir), runs it, prints stdout/stderr, and exits with the program's exit code. Clean up the temp binary afterward unless `--keep` is passed.

- [ ] **Step 4: Run to verify they pass** → PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/otter-driver/src/main.rs
git commit -m "feat(tooling): otter run, compile-and-run in one step"
```

---

### Task 4: `otter init` — project scaffolding

**Files:**
- Create: `crates/otter-driver/src/init.rs`
- Create: `crates/otter-driver/tests/init.rs`
- Modify: `crates/otter-driver/src/main.rs` (add `init` subcommand)

**Interfaces:**
- Consumes: a directory path
- Produces: `otter.toml`, `src/main.ot`, `.gitignore`, `README.md`

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn init_creates_a_valid_otter_toml() {}
#[test]
fn init_creates_a_compilable_main_ot() {}
#[test]
fn init_does_not_overwrite_an_existing_directory() {}
```

- [ ] **Step 2: Run to verify they fail** → FAIL.

- [ ] **Step 3: Implement**

Write a minimal `otter.toml` (`[package]`, name, version), a `src/main.ot` that prints `"Hello, Otter!"`, a `.gitignore` that ignores `target/`, and a `README.md` with build instructions.

- [ ] **Step 4: Run to verify they pass** → PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/otter-driver/src/init.rs crates/otter-driver/tests/init.rs crates/otter-driver/src/main.rs
git commit -m "feat(tooling): otter init, project scaffolding"
```

---

### Task 5: `otter doc` — documentation generator

**Files:**
- Create: `crates/otter-driver/src/doc.rs`
- Create: `crates/otter-driver/tests/doc.rs`
- Modify: `crates/otter-driver/src/main.rs` (add `doc` subcommand)

**Interfaces:**
- Consumes: source files, `///` doc comments
- Produces: HTML or Markdown documentation

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn doc_extracts_function_comments() {}
#[test]
fn doc_renders_types_and_signatures() {}
#[test]
fn doc_output_is_valid_html_or_markdown() {}
```

- [ ] **Step 2: Run to verify they fail** → FAIL.

- [ ] **Step 3: Implement**

Parse source files, extract `///` doc comments attached to function and type declarations, render them as HTML or Markdown with signatures.

- [ ] **Step 4: Run to verify they pass** → PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/otter-driver/src/doc.rs crates/otter-driver/tests/doc.rs crates/otter-driver/src/main.rs
git commit -m "feat(tooling): otter doc, documentation generator"
```

---

### Task 6: Language Server Protocol (LSP)

**Files:**
- Create: `crates/otter-lsp/src/lib.rs`
- Create: `crates/otter-lsp/src/server.rs`
- Create: `crates/otter-lsp/tests/lsp.rs`
- Modify: `crates/otter-driver/Cargo.toml` (add otter-lsp dependency or separate binary)

**Interfaces:**
- Consumes: `tower-lsp` protocol messages
- Produces: diagnostics, go-to-definition, hover, completion

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn lsp_reports_diagnostics_for_a_type_error() {}
#[test]
fn lsp_goes_to_definition_on_identifier_hover() {}
#[test]
fn lsp_completes_builtin_names() {}
```

- [ ] **Step 2: Run to verify they fail** → FAIL.

- [ ] **Step 3: Implement**

A `tower-lsp` server that:
- Opens a document, parses and type-checks it, publishes diagnostics with spans.
- On `textDocument/definition`, resolves the identifier under the cursor to its declaration.
- On `textDocument/hover`, shows the type of the symbol.
- On `textDocument/completion`, suggests builtins and in-scope names.

- [ ] **Step 4: Run to verify they pass** → PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/otter-lsp/ crates/otter-driver/Cargo.toml
git commit -m "feat(tooling): language server protocol support"
```

---

### Task 7: Syntax highlighting

**Files:**
- Create: `docs/highlighting/otter.sublime-syntax` (or `tree-sitter-otter/grammar.js`)
- Create: `docs/highlighting/README.md`
- Modify: `README.md` (editor setup section)

**Interfaces:**
- Consumes: Otter source files
- Produces: syntax highlighting rules for editors

- [ ] **Step 1: Write the failing tests**

```text
Manual: open an .ot file in VS Code / Sublime Text and verify keywords, strings, and comments are highlighted.
```

- [ ] **Step 2: Run to verify they fail** → FAIL.

- [ ] **Step 3: Implement**

A syntax definition covering:
- Keywords: `fn`, `const`, `var`, `mut`, `if`, `else`, `while`, `for`, `switch`, `case`, `default`, `return`, `break`, `continue`, `defer`, `move`, `assert`, `panic`, `unsafe`, `extern`
- Types: `i64`, `f64`, `bool`, `string`, `Option`, `Result`
- Strings, comments, numbers, operators

- [ ] **Step 4: Run to verify they pass** → PASS.

- [ ] **Step 5: Commit**

```bash
git add docs/highlighting/ README.md
git commit -m "feat(tooling): syntax highlighting for Otter"
```

---

### Task 8: One-command installer

**Files:**
- Create: `install.sh` (shell script, repo root)
- Create: `docs/install.md`
- Modify: `.github/workflows/*.yml` (publish prebuilt artifacts)

**Interfaces:**
- Consumes: nothing
- Produces: an `otter` binary on PATH

- [ ] **Step 1: Write the failing tests**

```text
Manual: curl ... | sh on a fresh machine produces a working `otter` binary.
```

- [ ] **Step 2: Run to verify they fail** → FAIL.

- [ ] **Step 3: Implement**

A shell script that detects OS/architecture, downloads a prebuilt `otter` binary from GitHub releases (or builds from source if no release exists), places it in `~/.local/bin` or `~/bin`, and adds that directory to PATH if needed.

- [ ] **Step 4: Run to verify they pass** → PASS.

- [ ] **Step 5: Commit**

```bash
git add install.sh docs/install.md
git commit -m "feat(tooling): one-command installer"
```

---

### Task 9: Error message quality pass

**Files:**
- Modify: `crates/otter-common/src/diagnostic.rs`
- Modify: `crates/otter-sema/src/error.rs`
- Modify: `crates/otter-sema/src/check_expr.rs`
- Modify: `crates/otter-parser/src/error.rs`

**Interfaces:**
- Consumes: existing diagnostics
- Produces: diagnostics with spans, carets, and helpful suggestions

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn diagnostic_renders_caret_under_offending_token() {}
#[test]
fn diagnostic_suggests_a_fix_for_immutable_assignment() {}
#[test]
fn diagnostic_names_expected_and_found_types() {}
```

- [ ] **Step 2: Run to verify they fail** → FAIL.

- [ ] **Step 3: Implement**

Review every diagnostic in the compiler. Each must:
- Name the file, line, and column.
- Show the source line with a `^` caret under the offending span.
- Suggest the most likely fix (e.g., "use `mut x` if you intend to mutate", "did you mean `assert_eq`?").
- Never panic or produce a raw Rust backtrace on malformed input.

- [ ] **Step 4: Run to verify they pass** → PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/otter-common/src/diagnostic.rs crates/otter-sema/src/error.rs crates/otter-sema/src/check_expr.rs crates/otter-parser/src/error.rs
git commit -m "feat(diagnostics): error message quality pass"
```

---

## Integration Checklist

Before M11 is considered complete:

- [ ] `cargo check --workspace --all-targets` is warning-free.
- [ ] `cargo test --workspace --no-fail-fast` is green (modulo the documented Device Guard flake).
- [ ] `otter fmt`, `otter test`, `otter run`, `otter init`, `otter doc` all compile and run.
- [ ] The language server starts and responds to `initialize`, `textDocument/didOpen`, and `textDocument/diagnostic`.
- [ ] The tutorial at `docs/tutorial/` can be followed from `curl ... | sh` to a running program in under 10 minutes.
- [ ] A fresh machine with no Rust toolchain can install Otter with one command.

---
