# Yashy Rename and Surface Simplification — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rename the language and every artefact from Otter to Yashy, then shrink the surface to one integer type, one float type, a `char`, `void`, one print family, one reassignable binding, one free method, and two array spellings — without ever regressing the existing suite.

**Architecture:** Six phases, each leaving a working language. Phase 1 changes only names; it must not move a single byte of program output, and the golden IR diff is the proof. Phases 2–5 each change one surface family in three steps: make the *new* spelling work, make the *old* spelling produce a diagnostic that names the replacement, then migrate the repository's own programs and tests. Phase 6 teaches the result. No phase depends on a later one, and every task ends with the workspace check and the test suite green.

**Tech Stack:** Rust (edition 2021, the toolchain in `rust-toolchain.toml`), textual LLVM IR through `clang`, the C runtime shim in `crates/yashy-runtime`, `cargo test`, PowerShell 7 for the bulk renames (`pwsh`, already required by `benchmarks/run.ps1`).

**Spec:** `docs/superpowers/specs/2026-10-23-yashy-rename-design.md`.

## Global Constraints

- The compiler never panics on user input: no `unwrap`, `expect`, `panic!`, `unreachable!`, or unchecked indexing in compiler code. A malformed program produces a diagnostic with a span and a caret.
- No LLVM build dependency; `clang`/`llc` stay subprocesses.
- Every task ends with `cargo check --workspace --all-targets` warning-free and `cargo test --workspace --no-fail-fast` passing.
- Device Guard / Smart App Control refusals (os error 4551) are environmental: `crates/yashy-driver/tests/e2e.rs` retries a refused launch, and a refused launch is never counted as a pass. If the test binary itself is refused, touch it so Cargo relinks it: `(Get-Item <test-exe>).LastWriteTime = Get-Date`.
- `cargo fmt` is not part of the gate. Do not run it. Clippy is blocked by host policy; do not run it.
- Golden IR changes are reviewed as deliberate diffs, never regenerated unread.
- Conventional commits; push to `origin main` after each task. Stage explicitly when the task touches a known file list; `git add -A` is allowed only after that task's full gate is green, so nothing untracked or stray can ride along.
- One spelling per operation. A migration diagnostic is not an alias: the old spelling must stop working.
- Dated records under `docs/superpowers/plans/` and `docs/superpowers/specs/2026-10-16-*` are history and are never rewritten. The 2026-10-23 spec is the only exception, and only when it is factually wrong.
- Every end-to-end program needs a named `#[test]` in `crates/yashy-driver/tests/e2e.rs` that builds and runs it. Adding only `tests/programs/name.ya` runs nothing.

## Environment Notes

- Run Cargo through the toolchain the session was set up with (the host's stable GNU toolchain is blocked by Application Control). On this machine that is `nightly-x86_64-pc-windows-gnu` with `CARGO_TARGET_DIR` under the session temp directory; the commands below say `cargo` and assume that resolution.
- The rename commands use PowerShell 7. `Set-Content -Encoding utf8NoBOM` is required: the repository is UTF-8 without BOM. Never let a bulk rewrite turn a file into UTF-16.
- `git mv` preserves history; plain `Move-Item` does not. Use `git mv`.

## Review Focus

1. **The rename smuggles in a behaviour change.** Phase 1 must be provably mechanical. The check is the golden IR diff (only `otter_`→`yashy_` and `.ot`→`.ya`) plus an unchanged e2e pass count. A "while I was there" edit in phase 1 is a defect.
2. **`char` compares with signed instructions.** `char` is 0–255. `'\xFF' > 'A'` must be true, which means unsigned predicates (`icmp ugt`), not the signed ones every other integer uses. A test must cross 0x7F.
3. **`void` leaks into value positions.** `let x: void`, `Option[void]`, `[]void` and a `void` parameter must all be refused, while `fn f() -> void` and `fn f()` remain identical. The existing `is_void_value`/`is_return` machinery is the place to enforce it, not a new flag on every call site.
4. **`println` allocates per call.** The newline must not be built by concatenating at run time. `println(x)` lowers to one `yashy_print_line_str` call; `println()` to the same call with an empty string.
5. **A removed builtin falls through to "unknown function".** `print_str(x)` must say it was replaced and name `println`, with a caret, not `cannot find function`.
6. **The view message regresses.** `[T]` is already refused; the change is the wording, and the range expression `a[start..end]` must keep working and keep being inferrable.
7. **The tutorial is a test, not prose.** It is migrated with each phase, not at the end: the harness compiles every block and compares its `// =>` lines, so a stale chapter fails the gate in the phase that changed the surface.

## File-Structure Map

| Area | Files | Phase |
|---|---|---|
| Workspace identity | `Cargo.toml`, `Cargo.lock`, `fuzz/Cargo.toml`, `fuzz/Cargo.lock`, all `crates/*/Cargo.toml` | 1.1 |
| Crate directories | `crates/otter-*` → `crates/yashy-*` (10) | 1.1 |
| CLI | `crates/yashy-driver/src/main.rs`, `Cargo.toml` `[[bin]]` | 1.2 |
| Test harnesses | `crates/yashy-driver/tests/{e2e,tutorial,stdlib,link,leaks,modules}.rs` | 1.2, 1.4 |
| Program corpus | 116 `*.ot` → `*.ya` under `crates/*/tests/`, `benchmarks/`, `crates/yashy-stdlib/yashy/` | 1.2, 2.1, 3.5, 4.1, 5.1, 5.2 |
| Runtime | `crates/yashy-runtime/{runtime.c,hashmap.c,include/yashy_runtime.h,build.rs}`, `src/lib.rs` | 1.3, 3.3, 4.1 |
| Lexer | `crates/yashy-lexer/src/{lexer.rs,token.rs}` | 1.2, 3.1, 5.1 |
| Parser | `crates/yashy-parser/src/{ast.rs,types.rs,exprs.rs,stmts.rs}` | 2.2, 3.1, 5.1 |
| Checker | `crates/yashy-sema/src/{check_expr.rs,ty.rs,const_eval.rs,hir.rs}`, `check_expr/debug.rs` | 2, 3, 4, 5 |
| Backend | `crates/yashy-codegen/src/{expr_gen.rs,ir.rs,strings.rs,lib.rs}` | 1.3, 3.2, 4.1 |
| Golden IR | `crates/yashy-codegen/tests/golden/*.{ya,expected.ll}` (12) | 1.2, 1.3, 3.2, 4.4 |
| Editors / CI | `editors/{vim,emacs,vscode}/**`, `.github/workflows/ci.yml`, `.gitattributes` | 1.4 |
| Docs | `README.md`, `docs/tutorial/*.md`, `docs/tutorial/run.sh`, `docs/performance-improvements.md` | 1.4, 2.1, 4.1, 4.4, 6.1, 6.2 |
| Benchmarks | `benchmarks/*.{ya,ps1,md}`, `benchmarks/features/*.ya` | 1.4, 6.3 |

---

## Phase 1 — The rename, and nothing else

### Task 1.1: Rename the ten crates

**Files:**
- Modify: `Cargo.toml` (workspace members, package names)
- Move: `crates/otter-{common,lexer,parser,sema,codegen,driver,runtime,fmt,lsp,stdlib}` → `crates/yashy-*`
- Move: `crates/yashy-runtime/include/otter_runtime.h` → `include/yashy_runtime.h` (the file's name follows its crate; the guard is Task 1.2's `OTTER_` pass)
- Modify: each `crates/*/Cargo.toml`, `fuzz/Cargo.toml`, `Cargo.lock`, `fuzz/Cargo.lock`
- Modify: every `*.rs` that names a crate (`use otter_common::…`)

**Interfaces:**
- Produces: crate paths `yashy_common`, `yashy_lexer`, `yashy_parser`, `yashy_sema`, `yashy_codegen`, `yashy_driver`, `yashy_runtime`, `yashy_fmt`, `yashy_lsp`, and directories `crates/yashy-*`; the fuzz package becomes `yashy-fuzz`.
- Consumes: nothing. This task changes no type, function, or diagnostic.

- [ ] **Step 1: Record the baseline**

This task has no new test: the existing suite *is* the test. Record it before touching anything.

```powershell
cargo test --workspace --no-fail-fast 2>&1 | Tee-Object -FilePath "$env:TEMP\pre-rename-tests.log"
Select-String -Path "$env:TEMP\pre-rename-tests.log" -Pattern 'test result:'
```

Expected, as measured on 2026-10-23: **84** `test result:` lines; all green except the suites the host's Application Control policy blocks with os error 4551. That run produced `otter-driver --test e2e` 95 passed / 9 failed, `--test leaks` 9 failed, `--test modules` 1, `--test tutorial` 1, and `otter-runtime --test racy_strings` 1 — every failure the same 4551 refusal, and every failing test passed when rerun alone. Record that baseline in the commit message; the gate for later tasks compares against it, and a 4551 failure is rerun rather than counted.

- [ ] **Step 2: Move the directories**

```powershell
$names = 'common','lexer','parser','sema','codegen','driver','runtime','fmt','lsp','stdlib'
foreach ($n in $names) { git mv "crates/otter-$n" "crates/yashy-$n" }
git mv crates/yashy-runtime/include/otter_runtime.h crates/yashy-runtime/include/yashy_runtime.h
git mv crates/yashy-stdlib/otter crates/yashy-stdlib/yashy
Get-ChildItem crates -Directory | Select-Object -ExpandProperty Name
```

Expected: exactly the ten `yashy-*` directories; no `otter-*` remains.

Two moves beyond the ten crates belong here. The header moves because Step 3's crate-identifier rewrite turns every `otter_runtime` into `yashy_runtime` — including the `include_str!` path and `compile("otter_runtime")` — so the file must already have that name or this task cannot compile. The stdlib's inner module directory moves because it is a path component, not an identifier: no content rewrite renames a directory.

- [ ] **Step 3: Rewrite the crate identifiers**

Only the crate identifier changes in this step. The runtime symbols (`otter_print_str`) and the binary (`otter`) are separate tasks, so a pattern restricted to the ten known suffixes is what keeps this step from doing too much.

```powershell
Get-ChildItem -Recurse -File -Include *.rs,*.toml,*.c,*.h |
  Where-Object { $_.FullName -notmatch '\\target\\|\\.git\\|\\benchmarks\\build\\' } |
  ForEach-Object {
    $path = $_.FullName
    $text = [System.IO.File]::ReadAllText($path)
    $next = $text -creplace 'otter_(common|lexer|parser|sema|codegen|driver|runtime|fmt|lsp|stdlib)','yashy_$1'
    $next = $next -creplace 'otter-(common|lexer|parser|sema|codegen|driver|runtime|fmt|lsp|stdlib|fuzz)','yashy-$1'
    $next = $next -creplace '\.join\("otter"\)','.join("yashy")'
    $next = $next -creplace 'otter_runtime\.h','yashy_runtime.h'
    if ($next -cne $text) { [System.IO.File]::WriteAllText($path, $next, (New-Object System.Text.UTF8Encoding($false))) }
  }
Select-String -Path Cargo.toml -Pattern 'yashy'
```

Two details are load-bearing in that snippet. `-creplace`, not `-replace`: PowerShell's operator ignores case by default, and `-replace 'otter_(runtime)'` would rewrite the header guard `OTTER_RUNTIME_H` into `yashy_RUNTIME_H`. And `[System.IO.File]::WriteAllText` with a BOM-less `UTF8Encoding`, because this host's `powershell` is 5.1, where `Set-Content -Encoding utf8NoBOM` does not exist and fails the whole write. The `.c`/`.h` files are in scope because `runtime.c` and `hashmap.c` `#include "otter_runtime.h"`; the crate-suffix patterns cannot touch a symbol like `otter_print_str`.

Expected: `Cargo.toml` lists `crates/yashy-common` … `crates/yashy-lsp`; no `otter` remains in any manifest. The third replacement is for `crates/yashy-driver/tests/stdlib.rs`, which builds the stdlib path as `.join("otter-stdlib").join("otter")` — the first two replacements catch `otter-stdlib`, only this one catches the trailing component.

`fuzz/` is its own workspace with its own lock file; regenerate it so it stops naming the old packages:

```powershell
cargo metadata --manifest-path fuzz/Cargo.toml --format-version 1 > $null
Select-String -Path fuzz/Cargo.lock -Pattern 'otter'
```

Expected: the second command prints nothing. `fuzz/Cargo.lock` is ignored by `fuzz/.gitignore` and is therefore regenerated locally but never staged.

- [ ] **Step 4: Check and test**

```powershell
cargo check --workspace --all-targets
cargo test --workspace --no-fail-fast
```

Expected: `cargo check` produces no warnings; the same 84 suites and the same pass count as Step 1 (modulo 4551 reruns).

- [ ] **Step 5: Commit**

```powershell
git add Cargo.toml Cargo.lock fuzz/Cargo.toml crates
git commit -m "refactor: rename the crates from otter-* to yashy-*"
git push origin main
```

`fuzz/Cargo.lock` is not in that list on purpose: `fuzz/.gitignore` line 5 ignores `Cargo.lock`, so staging it would fail with "paths are ignored". It is regenerated locally so the fuzz workspace resolves the new package names, and it stays untracked exactly as it was before this rename.

The body records the baseline counts: `N suites, M tests, unchanged.`

---

### Task 1.2: Rename the binary, the file extension, the environment, and the temp directories

**Files:**
- Modify: `crates/yashy-driver/Cargo.toml` (`[[bin]] name`)
- Modify: `crates/yashy-driver/src/main.rs` (usage text, `version` line)
- Modify: `crates/yashy-driver/tests/{e2e,tutorial,stdlib,link,leaks,modules}.rs` (`CARGO_BIN_EXE_otter` → `CARGO_BIN_EXE_yashy`, helper `fn otter()` → `fn yashy()`)
- Modify: `crates/yashy-driver/src/{toolchain,pipeline,modules}.rs` (env constants)
- Move: every `*.ot` → `*.ya` (116 files)
- Modify: `.gitattributes`, `docs/tutorial/run.sh`, `benchmarks/*.ps1`
- Test: `crates/yashy-driver/tests/e2e.rs`

**Interfaces:**
- Produces: the binary `yashy`; sources end in `.ya`; the 17 `OTTER_*` settings read `YASHY_*`; temp directories are `yashy-*`.
- Consumes: Task 1.1's crate paths.
- Note: the CLI does not validate a source file's extension. The extension is a convention carried by tests, docs and editor rules, so no parser change is needed.

- [ ] **Step 1: Write the failing test**

In `crates/yashy-driver/tests/e2e.rs`, change the existing version test to assert the new name:

```rust
#[test]
fn version_prints_a_version_and_exits_zero() {
    let output = Command::new(yashy())
        .arg("version")
        .output()
        .expect("run the yashy CLI");
    assert!(output.status.success());
    let stdout = stdout_of(&output);
    assert!(stdout.starts_with("yashy "), "no version line:\n{stdout}");
}
```

- [ ] **Step 2: Run it and verify it fails**

```powershell
cargo test -p yashy-driver --test e2e version_prints_a_version_and_exits_zero
```

Expected: FAIL — `no version line: otter 0.1.0`.

- [ ] **Step 3: Rename the binary**

In `crates/yashy-driver/Cargo.toml`:

```toml
[[bin]]
name = "yashy"
path = "src/main.rs"
```

In `crates/yashy-driver/src/main.rs`: replace `otter` with `yashy` in the version `println!` only — the module doc and `USAGE` are prose and belong to Task 1.4, whose test asserts them. In the six test files, replace `CARGO_BIN_EXE_otter` with `CARGO_BIN_EXE_yashy` and rename the helper `fn otter()` to `fn yashy()` at every call site.

- [ ] **Step 4: Rename the programs and their references**

```powershell
Get-ChildItem -Recurse -Filter *.ot -File |
  Where-Object { $_.FullName -notmatch '\\target\\|\\.git\\' } |
  ForEach-Object { git mv -- $_.FullName ($_.FullName -replace '\.ot$','.ya') }

Get-ChildItem -Recurse -File -Include *.rs,*.md,*.sh,*.ps1,*.json,*.vim,*.el,*.yml |
  Where-Object { $_.FullName -notmatch '\\target\\|\\.git\\|\\.superpowers\\' -and
                 $_.FullName -notmatch '\\docs\\superpowers\\' } |
  ForEach-Object {
    $path = $_.FullName; $text = Get-Content -Raw -LiteralPath $path
    $next = $text -replace '\.ot\b','.ya'
    if ($next -ne $text) { Set-Content -LiteralPath $path -Value $next -NoNewline -Encoding utf8NoBOM }
  }
```

Add the `.ya` patterns to `.gitattributes` beside the `.ot` ones (`**/tests/golden/*.ya`, `**/tests/programs/*.ya`, both `text eol=lf`) and delete the `.ot` patterns: no `.ot` file will exist.

- [ ] **Step 5: Rename the environment variables and temp directories**

```powershell
Get-ChildItem -Recurse -File -Include *.rs,*.md,*.ps1,*.sh,*.yml,*.c,*.h |
  Where-Object { $_.FullName -notmatch '\\target\\|\\.git\\|\\.superpowers\\' -and
                 $_.FullName -notmatch '\\docs\\superpowers\\' } |
  ForEach-Object {
    $path = $_.FullName; $text = Get-Content -Raw -LiteralPath $path
    $next = $text -replace 'OTTER_','YASHY_' -replace 'otter-([a-z])','yashy-$1'
    if ($next -ne $text) { Set-Content -LiteralPath $path -Value $next -NoNewline -Encoding utf8NoBOM }
  }
Select-String -Path crates/yashy-driver/src/toolchain.rs -Pattern 'YASHY_LLC'
```

Expected: `pub const LLC_ENV_VAR: &str = "YASHY_LLC";`.

The dashed rule is generic on purpose. Beyond the `yashy-*` crates (already renamed by 1.1, so no longer matching), the sources name temp directories by language, and they are not all obvious: `otter-e2e`, `otter-tutorial`, `otter-link`, `otter-modules`, `otter-leaks`, `otter-racy`, `otter-test`, `otter-threads`, `otter-path`, `otter-cache`. A list of alternatives was tried on paper and missed five of them; `otter-([a-z])` catches every one and cannot touch a symbol (`otter_print_str` is not dashed).

- [ ] **Step 6: Regenerate the goldens and prove the diff is only names**

The goldens embed the source path and the runtime symbols, so they change here (path/extension) and again in Task 1.3 (symbols).

```powershell
$before = "$env:TEMP\golden-1.2"
Copy-Item crates/yashy-codegen/tests/golden $before -Recurse -Force
$env:UPDATE_GOLDEN = "1"; cargo test -p yashy-codegen
Remove-Item Env:\UPDATE_GOLDEN
Get-ChildItem crates/yashy-codegen/tests/golden -Filter *.expected.ll | ForEach-Object {
  $old = (Get-Content -Raw (Join-Path $before $_.Name)) -replace '\.ot\b','.ya'
  if ($old -ne (Get-Content -Raw $_.FullName)) { "DIFF: $($_.Name)" }
}
```

Expected: no `DIFF:` lines.

- [ ] **Step 7: Run the focused and workspace tests**

```powershell
cargo test -p yashy-driver --test e2e version_prints_a_version_and_exits_zero
cargo test --workspace --no-fail-fast
```

Expected: the version test PASSes with `yashy 0.1.0`; the suite count and totals match Task 1.1.

- [ ] **Step 8: Commit**

```powershell
git add -A
git commit -m "refactor: rename the binary to yashy and sources to .ya"
git push origin main
```

---

### Task 1.3: Rename the runtime symbols

**Files:**
- Modify: `crates/yashy-runtime/runtime.c`, `hashmap.c`, `include/yashy_runtime.h`, `build.rs`, `src/lib.rs`
- Modify: every codegen string that names a symbol (`crates/yashy-codegen/src/**`)
- Modify: `crates/yashy-common`, `crates/yashy-driver/src/pipeline.rs` if they embed the header

**Interfaces:**
- Produces: all exported C symbols `yashy_*`; the static library is `yashy_runtime`; the embedded header is `yashy_runtime.h` with `yashy_str`, `yashy_print_str`, etc.
- Consumes: nothing from a later task.

- [ ] **Step 1: Rename every symbol**

After Task 1.1, the only `otter_` left in Rust source is a runtime symbol inside a string or a raw identifier, so the replacement is safe in one pass.

```powershell
Get-ChildItem -Recurse -File -Include *.c,*.h,*.rs |
  Where-Object { $_.FullName -notmatch '\\target\\|\\.git\\' } |
  ForEach-Object {
    $path = $_.FullName; $text = Get-Content -Raw -LiteralPath $path
    $next = $text -replace 'otter_','yashy_'
    if ($next -ne $text) { Set-Content -LiteralPath $path -Value $next -NoNewline -Encoding utf8NoBOM }
  }
```

The first pass already turns `otter_str` into `yashy_str` (it carries the `otter_` prefix) and rewrites the `include_str!` path. Only the Rust mirror `OtterStr` has no underscore and needs its own pass:

```powershell
Get-ChildItem crates/yashy-runtime -Recurse -File -Include *.rs,*.h,*.c |
  ForEach-Object {
    $path = $_.FullName; $text = Get-Content -Raw -LiteralPath $path
    $next = $text -replace 'OtterStr','YashyStr'
    if ($next -ne $text) { Set-Content -LiteralPath $path -Value $next -NoNewline -Encoding utf8NoBOM }
  }
```

Rename the static library in `build.rs` (`compile("yashy_runtime")`) and its header path (`include/yashy_runtime.h`). The `rustc-env` lines become `YASHY_TARGET`/`YASHY_HOST`; Step 5 of Task 1.2 already renamed the readers.

- [ ] **Step 2: Check nothing was missed**

```powershell
git grep -n "otter_" -- crates fuzz Cargo.toml
```

Expected: no output.

- [ ] **Step 3: Regenerate the goldens and prove the diff is only symbols**

```powershell
$before = "$env:TEMP\golden-1.3"
Copy-Item crates/yashy-codegen/tests/golden $before -Recurse -Force
$env:UPDATE_GOLDEN = "1"; cargo test -p yashy-codegen
Remove-Item Env:\UPDATE_GOLDEN
Get-ChildItem crates/yashy-codegen/tests/golden -Filter *.expected.ll | ForEach-Object {
  $old = (Get-Content -Raw (Join-Path $before $_.Name)) -replace 'otter_','yashy_'
  if ($old -ne (Get-Content -Raw $_.FullName)) { "DIFF: $($_.Name)" }
}
```

Expected: no `DIFF:` lines.

- [ ] **Step 4: Run the runtime and end-to-end tests**

```powershell
cargo test -p yashy-runtime
cargo test -p yashy-driver --test e2e hello
cargo test --workspace --no-fail-fast
```

Expected: the runtime tests PASS (the C and the Rust declarations agree), `hello` prints its line, and the suite totals are unchanged.

- [ ] **Step 5: Commit**

```powershell
git add -A
git commit -m "refactor: rename the runtime symbols and header to yashy_*"
git push origin main
```

---

### Task 1.4: Rename the prose, editors, CI, and the tutorial's fences

**Files:**
- Modify: `README.md`, `docs/performance-improvements.md`, `docs/tutorial/*.md`, `docs/tutorial/run.sh`
- Modify: `editors/vim/*`, `editors/emacs/*`, `editors/vscode/**` (file names and contents)
- Modify: `.github/workflows/ci.yml`
- Modify: `crates/yashy-driver/tests/tutorial.rs` (fence marker)
- Test: `crates/yashy-driver/tests/e2e.rs`

**Interfaces:**
- Produces: no live file names the old language; tutorial fences are ```` ```yashy ````; the tutorial harness looks for that marker.
- Consumes: Tasks 1.2–1.3.

**Do not touch:** `docs/superpowers/plans/**`, `docs/superpowers/specs/2026-10-16-otter-language-design.md`, `docs/superpowers/specs/2026-10-23-yashy-rename-design.md`, `.superpowers/**`.

- [ ] **Step 1: Write the failing test**

Add to `crates/yashy-driver/tests/e2e.rs`:

```rust
#[test]
fn the_help_text_names_yashy_and_the_ya_extension() {
    let output = Command::new(yashy())
        .arg("--help")
        .output()
        .expect("run the yashy CLI");
    assert!(output.status.success());
    let stdout = stdout_of(&output);
    assert!(stdout.contains("yashy build <file.ya>"), "help:\n{stdout}");
    assert!(!stdout.contains("otter"), "the old name survives:\n{stdout}");
}
```

- [ ] **Step 2: Run it and verify it fails**

```powershell
cargo test -p yashy-driver --test e2e the_help_text_names_yashy_and_the_ya_extension
```

Expected: FAIL — the help still says `` otter build <file.ya> `` (Task 1.2 renamed only the version line).

- [ ] **Step 3: Rename the editors and CI**

```powershell
git mv editors/vim/otter.vim editors/vim/yashy.vim
git mv editors/emacs/otter.el editors/emacs/yashy.el
git mv editors/vscode/syntaxes/otter.tmLanguage.json editors/vscode/syntaxes/yashy.tmLanguage.json

Get-ChildItem -Recurse -File -Include *.rs,*.toml,*.md,*.sh,*.ps1,*.yml,*.yaml,*.json,*.vim,*.el,*.c,*.h,*.ya,*.txt,.gitattributes |
  Where-Object { $_.FullName -notmatch '\\target\\|\\.git\\|\\.superpowers\\|\\benchmarks\\build\\|\\fuzz\\corpus\\' -and
                 $_.FullName -notmatch '\\docs\\superpowers\\' } |
  ForEach-Object {
    $path = $_.FullName; $text = Get-Content -Raw -LiteralPath $path
    $next = $text -replace 'Otter','Yashy' -replace 'OTTER','YASHY' -replace 'otter','yashy'
    if ($next -ne $text) { Set-Content -LiteralPath $path -Value $next -NoNewline -Encoding utf8NoBOM }
  }
```

This pass also renames `.ot` inside old prose to `.ya` only if Step 4 of Task 1.2 missed it; check the tutorial with `Select-String -Path docs/tutorial/*.md -Pattern '\.ot\b'`.

The extension list is not laziness: `benchmarks/build/` holds untracked `.exe` files and `fuzz/corpus/` holds arbitrary bytes, both of which a text rewrite would corrupt. Prose in a `.md`, a `.rs`, or a manifest is what this pass is for.

- [ ] **Step 4: Convert the tutorial fences**

```powershell
Get-ChildItem docs/tutorial -Filter *.md | ForEach-Object {
  $path = $_.FullName; $text = Get-Content -Raw -LiteralPath $path
  $next = $text -replace '```otter','```yashy'
  if ($next -ne $text) { Set-Content -LiteralPath $path -Value $next -NoNewline -Encoding utf8NoBOM }
}
```

In `crates/yashy-driver/tests/tutorial.rs`, change the fence test from `starts_with("```otter")` to `starts_with("```yashy")` and update the module doc that says "```otter block".

- [ ] **Step 5: Add the former-name note to the README**

Immediately under the title:

```markdown
> **Formerly Otter.** The language, binary, crates and file extension were
> renamed on 2026-10-23; dated design records under `docs/superpowers/` keep the
> old name because they are history.
```

- [ ] **Step 6: Prove no live file names the old language**

```powershell
git grep -n -i otter -- ':!docs/superpowers' ':!.superpowers'
```

Expected: no output except the new README note. If anything else appears, fix it; a comment that says "an Otter pointer" is still a live name.

- [ ] **Step 7: Run the tests**

```powershell
cargo test -p yashy-driver --test e2e the_help_text_names_yashy_and_the_ya_extension
cargo test -p yashy-driver --test tutorial
cargo test --workspace --no-fail-fast
```

Expected: PASS. The tutorial harness compiles every ```` ```yashy ```` block and matches its `// =>` lines; the totals match Task 1.1.

- [ ] **Step 8: Commit**

```powershell
git add -A
git commit -m "docs: rename the remaining prose, editors and CI to Yashy"
git push origin main
```

---

### Task 1, as built

Five things differed from the plan above; each is recorded because a later task
depends on the outcome.

1. **This host's `powershell` is 5.1.** `Set-Content -Encoding utf8NoBOM` does not
   exist there, and the first rewrite pass failed every write while the loop kept
   going. Every bulk rewrite used
   `[System.IO.File]::WriteAllText($path, $next, (New-Object System.Text.UTF8Encoding($false)))`
   instead. `-creplace` replaced `-replace` everywhere: PowerShell's default is
   case-insensitive, so `-replace 'otter_(runtime)','yashy_$1'` would have
   rewritten the header guard `OTTER_RUNTIME_H` into `yashy_RUNTIME_H`.

2. **The header move belongs to 1.1's rewrite.** `runtime.c` and `hashmap.c`
   `#include "otter_runtime.h"`, so the `.c`/`.h` files joined that scan; a
   crate-identifier pass over `.rs`/`.toml` alone cannot compile.

3. **Two `"ot"` spellings are not path suffixes.** `crates/yashy-driver/src/modules.rs`
   appends the extension with `set_extension("ot")`, and the lexer's golden test
   filters with `ext == "ot"`. A `\.ot\b` textual pass cannot see either; both
   are `"ya"` now.

4. **The runtime's Rust mirror is three types, not one.** Besides `OtterStr`,
   `OtterMap` and `OtterSlice` name the same layouts on the Rust side; all three
   became `Yashy*`.

5. **Two programs spell the language name in fragments.** `04-structs.md` and
   `tests/programs/struct_ownership.ya` build it as `"ott" + "er"`, so the prose
   pass changed the expected line but not the printed value. Both now build
   `"yas" + "hy"`. Likewise `benchmarks/loop.mjs` is a `.mjs` file, outside the
   sweep's extension list; it named the language and `loop.ot`, and now names
   neither.

The goldens were regenerated three times, once per kind of change, and each time
the diff was proven to be exactly the substitution: `.ot`→`.ya` in the embedded
source path, `otter_`→`yashy_` in the symbols, `Otter`→`Yashy` in the emitted
header comment.

`fuzz/corpus/lex_parse/hello` still contains `"Hello, Otter!"`. It is an
untouched corpus entry, not a name: rewriting test data would change the input
the corpus exists to preserve.

The only live occurrences of the old name left in the tree are the README's
former-name note and the help test's regression assertion, which has to name what
it forbids.

Gate: `cargo check --workspace --all-targets` is warning-free. The first
`cargo test --workspace --no-fail-fast` ran 84 test binaries and failed 130
tests, every one of them this host's Application Control refusal (os error 4551;
Smart App Control is on) rather than an assertion. Each failure was rerun and
every rerun passed: e2e 105/105, tutorial, modules, stdlib, link, `parser --doc`,
and all ten leak tests individually. The refusal fell on a freshly rebuilt
`yashy.exe` as well, so the evidence run rebuilt the driver alone and reused the
already-trusted binary; `cargo test -p yashy-driver -p yashy-runtime` relinks the
binary under a different feature set and loses that trust.

---

## Phase 2 — Type spellings

### Task 2.1: `int` and `float` replace `i64` and `f64`

**Files:**
- Modify: `crates/yashy-sema/src/check_expr.rs` (`resolve_type_in`, `type_to_expr`)
- Modify: `crates/yashy-sema/src/ty.rs` (`Display for TypeKind`)
- Test: `crates/yashy-sema/tests/check.rs`
- Test: `crates/yashy-driver/tests/e2e.rs`

**Interfaces:**
- Produces: `int` and `float` are the wide spellings; `TypeKind::I64`/`TypeKind::F64` keep their Rust names and their `Display` now reads `int`/`float`. A Yashy type position or an `as` target that says `i64`/`f64` is a `TypeError` naming the replacement.
- Consumes: nothing. `extern` declarations keep accepting `i64`/`f64` through `from_c_spelling`: an FFI signature is a C signature, and `int64_t`'s alias is not a Yashy type. Do not remove those two rows from `from_c_spelling`.

**Placement:** an `as` target resolves with `allow_c = true`, so rejecting only when `!allow_c` would leave `x as i64` working — a second spelling. Reject when `!allow_c` in the resolver, and add an explicit check in the cast arm (which is the only other caller with `allow_c = true`) so the cast target is refused too. `extern` is the one place that keeps the old names.

- [ ] **Step 1: Write the failing tests**

In `crates/yashy-sema/tests/check.rs`:

```rust
#[test]
fn int_and_float_are_the_wide_spellings() {
    let (result, _, _) = check_source("fn f(a: int) -> float { return 1.5; }");
    assert!(result.is_ok(), "int and float must resolve: {result:?}");
}

#[test]
fn i64_names_its_replacement() {
    assert_error("fn f(a: i64) { }", "`i64` is spelled `int` in Yashy");
}

#[test]
fn f64_names_its_replacement() {
    assert_error("fn f() -> f64 { return 1.0; }", "`f64` is spelled `float` in Yashy");
}

#[test]
fn a_cast_cannot_spell_the_old_names_either() {
    assert_error("fn f(x: int) { const y = x as i64; }", "`i64` is spelled `int` in Yashy");
}
```

And in `crates/yashy-driver/tests/e2e.rs`, a caret test modelled on `a_type_error_produces_a_caret_diagnostic_and_exit_1`:

```rust
#[test]
fn an_i64_annotation_is_reported_with_a_caret() {
    let dir = work_dir("old-int-spelling");
    let source = dir.join("old.ya");
    fs::write(&source, "fn main() { const n: i64 = 1; }\n").expect("write the program");
    let output = Command::new(yashy()).arg("check").arg(&source).output().expect("run check");
    assert_eq!(output.status.code(), Some(1));
    let stderr = stderr_of(&output);
    assert!(stderr.contains("`i64` is spelled `int` in Yashy"), "{stderr}");
    assert!(stderr.contains('^'), "expected a caret:\n{stderr}");
}
```

- [ ] **Step 2: Run them and verify they fail**

```powershell
cargo test -p yashy-sema --test check int_and_float_are_the_wide_spellings
cargo test -p yashy-sema --test check i64_names_its_replacement
```

Expected: the first FAILs with `cannot find type \`int\``; the second FAILs because `i64` still resolves.

- [ ] **Step 3: Implement the spellings and the diagnostics**

In `resolve_type_in`'s `TypeExpr::Name` arm, near the top:

```rust
if !allow_c && (name == "i64" || name == "f64") {
    let proper = if name == "i64" { "int" } else { "float" };
    return Err(TypeError::new(
        *span,
        format!("`{name}` is spelled `{proper}` in Yashy"),
    ));
}
```

In the match, add:

```rust
"int" => Some(TypeKind::I64),
"float" => Some(TypeKind::F64),
```

In `ty.rs`'s `Display`, `TypeKind::I64` writes `int` and `TypeKind::F64` writes `float`.

In `check_expr.rs`'s `type_to_expr` (the `TypeKind` → `TypeExpr` direction near line 7900), write `int`/`float`.

In the cast arm (`Expr::Cast`, near line 6723, which calls `resolve_or_report_in(ty, span, true, false)`), reject the two names before resolving:

```rust
if let TypeExpr::Name(name, span) = ty.as_ref() {
    if name == "i64" || name == "f64" {
        return self.error(*span, format!("`{name}` is spelled `{}` in Yashy",
            if name == "i64" { "int" } else { "float" }));
    }
}
```

Then sweep the diagnostics that hardcode the old names in prose, which `Display` cannot fix:

```powershell
git grep -n "i64\|f64" -- crates/*/src
```

Update the message strings and doc comments only (for example the map-key message at `check_expr.rs:1800`, which lists `i64`, `bool`, `string`, and the note above `is_c_only_spelling` that calls `i64`/`f64` ordinary types). Leave Rust types, `from_c_spelling`, and the C-width tables alone. `mangled_type_name`'s `"i64"`/`"f64"` fragments also stay: they are internal symbol pieces, and renaming them would churn every generic golden for no user-visible gain.

- [ ] **Step 4: Run the focused tests**

```powershell
cargo test -p yashy-sema --test check int_and_float_are_the_wide_spellings
cargo test -p yashy-sema --test check i64_names_its_replacement
cargo test -p yashy-sema --test check f64_names_its_replacement
cargo test -p yashy-sema --test check a_cast_cannot_spell_the_old_names_either
cargo test -p yashy-driver --test e2e an_i64_annotation_is_reported_with_a_caret
```

Expected: all PASS.

- [ ] **Step 5: Migrate the corpus and the live docs**

The suite is now red wherever a `.ya` or a tutorial block says `i64`/`f64`; this step is what makes the task end green. Word boundaries matter, and no `*.rs` file is touched — `i64::MAX` there is Rust, not Yashy.

```powershell
Get-ChildItem -Recurse -File -Include *.ya,*.md |
  Where-Object { $_.FullName -notmatch '\\target\\|\\.git\\|\\.superpowers\\' -and
                 $_.FullName -notmatch '\\docs\\superpowers\\' } |
  ForEach-Object {
    $path = $_.FullName; $text = Get-Content -Raw -LiteralPath $path
    $next = $text -replace '\bi64\b','int' -replace '\bf64\b','float'
    if ($next -ne $text) { Set-Content -LiteralPath $path -Value $next -NoNewline -Encoding utf8NoBOM }
  }
```

- [ ] **Step 6: Fix the Rust-side expectations**

```powershell
cargo test --workspace --no-fail-fast 2>&1 | Tee-Object "$env:TEMP\after-int.log"
Select-String -Path "$env:TEMP\after-int.log" -Pattern 'FAILED|panicked at|expected'
```

Each failure is one of two things: an expected message that still quotes `i64`/`f64`, or a test's *source string* that still writes them. Fix the message expectation to the compiler's new wording, and fix the source to `int`/`float`. Do not blank-replace `i64` across `*.rs`; update the failing lines only.

- [ ] **Step 7: Prove the corpus is clean**

```powershell
Get-ChildItem -Recurse -Filter *.ya | Select-String -Pattern '\bi64\b|\bf64\b'
```

Expected: no output.

- [ ] **Step 8: Run the full gate**

```powershell
cargo check --workspace --all-targets
cargo test --workspace --no-fail-fast
```

Expected: no warnings; every suite `ok.` with the new tests included.

- [ ] **Step 9: Commit**

```powershell
git add -A
git commit -m "feat: spell the wide types int and float"
git push origin main
```

---

### Task 2.2: `void` replaces `()`

**Files:**
- Modify: `crates/yashy-sema/src/check_expr.rs` (`resolve_type_in`, `resolve_or_report`, `is_void_value`, `type_expr_name`, `type_to_expr`, `mangled_type_name`)
- Modify: `crates/yashy-sema/src/ty.rs` (`Display`, `is_c_only_spelling`)
- Modify: `crates/yashy-parser/src/types.rs` (synthesize `void` for a function type's missing arrow)
- Test: `crates/yashy-sema/tests/check.rs`

**Interfaces:**
- Produces: `void` is the no-value spelling; it is legal only as a function's return type. `fn f()` and `fn f() -> void` are identical. A written `()` in type position is a checker error that names `void`.
- Consumes: the existing `is_void_value` + `is_return` machinery (`resolve_or_report_in`), which already enforces the rule for `extern`.

**Why the checker, not the parser:** the sema test harness (`check_source`) expects `parse_program()` to succeed and only inspects checker diagnostics, so a parse error cannot be asserted with `assert_error`. Rejecting `()` in `resolve_type_in`'s existing `TypeExpr::Unit` arm keeps the test simple and puts the diagnostic next to the other type errors.

- [ ] **Step 1: Write the failing tests**

In `crates/yashy-sema/tests/check.rs`:

```rust
#[test]
fn void_is_the_return_spelling() {
    let (result, _, _) = check_source("fn f() -> void { }");
    assert!(result.is_ok(), "`-> void` must be accepted: {result:?}");
}

#[test]
fn a_written_unit_type_names_void() {
    assert_error("fn f() -> () { }", "`()` is spelled `void` in Yashy");
}

#[test]
fn void_outside_a_return_position_is_refused() {
    assert_error("fn f(x: void) { }", "only allowed as a function's return type");
    assert_error("fn g() { } fn f() { const x: void = g(); }", "only allowed as a function's return type");
    assert_error("fn f(xs: []void) { }", "only allowed as a function's return type");
}
```

- [ ] **Step 2: Run them and verify they fail**

```powershell
cargo test -p yashy-sema --test check void_is_the_return_spelling
cargo test -p yashy-sema --test check a_written_unit_type_names_void
cargo test -p yashy-sema --test check void_outside_a_return_position_is_refused
```

Expected: `-> void` FAILs with `cannot find type \`void\``; `-> ()` FAILs because it is accepted; the placement test FAILs for the same reason.

- [ ] **Step 3: Implement `void`**

1. `ty.rs`: `Display` for `TypeKind::Unit` writes `void`; remove `"void"` from `is_c_only_spelling` (so the ordinary name match can see it; `from_c_spelling` still maps it to `CVoid` inside `extern`).
2. `check_expr.rs`, `resolve_type_in`'s name match: add `"void" => Some(TypeKind::Unit)` **only when `!allow_c`**; delete `"()" => Some(TypeKind::Unit)` (no source produces it once the `TypeExpr::Unit` arm reports).
3. `check_expr.rs`, the `TypeExpr::Unit(span)` arm of `resolve_type_in` (line ~1840): return `TypeError::new(*span, "`()` is spelled `void` in Yashy")` instead of `Ok(Unit)`.
4. `is_void_value`: add `TypeKind::Unit => true`, and recurse through `Slice`, `FixedArray` and `Ptr` in addition to `Option`/`Result`. Do **not** recurse into `Func`: a function type's return is a return position, so `fn() -> void` is legal.
5. Split the report helpers, keeping `resolve_or_report`'s visibility check:
   - `resolve_or_report` (line ~1527) keeps its body, and after `resolve_type` succeeds it refuses a void value with `` `void` is only allowed as a function's return type `` and returns `never`.
   - Add `resolve_or_report_return` with the same body **without** that refusal, and use it at the six return annotations: `check_expr.rs` lines 1012 (method signature), 1206 (function signature), 1491, 1979, 2182, 6816. The `extern` path (1448) already uses `resolve_or_report_in(…, true, true)`; update its message to the same wording.
6. The four plumbing sites that turn a `TypeKind::Unit` back into a name must agree: `type_expr_name` (line ~261) writes `"void"`; `mangled_type_name` (line ~181) writes `"void"` instead of `"unit"`; `type_to_expr` (line ~7903) and the `Never` fallback (line ~7946) return `TypeExpr::Name("void".to_string(), span)` rather than `TypeExpr::Unit(span)`, so no synthesized type can hit the new error.
7. `types.rs`: the function-type parser's missing arrow synthesizes `TypeExpr::Name("void".to_string(), close.span)` instead of `TypeExpr::Unit(close.span)`. A *written* `()` keeps parsing to `TypeExpr::Unit`; step 3 is what refuses it.

- [ ] **Step 4: Run the tests and the workspace check**

```powershell
cargo test -p yashy-sema --test check void_is_the_return_spelling
cargo test -p yashy-sema --test check a_written_unit_type_names_void
cargo test -p yashy-sema --test check void_outside_a_return_position_is_refused
cargo check --workspace --all-targets
```

Expected: all PASS, no warnings. Rust test strings that assert `()` in a message now fail; update each to `void` (search `assert_error` needles for `"()"`).

- [ ] **Step 5: Commit**

```powershell
git add -A
git commit -m "feat: spell the no-value type void"
git push origin main
```

---

## Phase 3 — `char`

### Task 3.1: Lex `'A'` into a `Char` token and a `Literal::Char`

**Files:**
- Modify: `crates/yashy-lexer/src/token.rs` (`TokenKind::Char`, naming, `char_literal_value`)
- Modify: `crates/yashy-lexer/src/lexer.rs` (`scan_char`)
- Modify: `crates/yashy-lexer/src/error.rs` (char-specific messages)
- Modify: `crates/yashy-parser/src/ast.rs` (`Literal::Char(u8)`)
- Modify: `crates/yashy-parser/src/exprs.rs` (literal expression)
- Test: inline `#[cfg(test)]` tests in `lexer.rs`; `crates/yashy-parser/tests/parse.rs`

**Interfaces:**
- Produces: `TokenKind::Char`; `yashy_lexer::char_literal_value(src: &str, span: Span, file: FileId) -> Result<u8, Diagnostic>` (the same shape as `literal_as_i64`); `Literal::Char(u8)`.
- Consumes: `token::decode_escape` (make it `pub(crate)` if it is private).

- [ ] **Step 1: Write the failing lexer tests**

```rust
/// The byte a lone char literal stands for, in a file of its own.
fn char_value(src: &str) -> Result<u8, Diagnostic> {
    let mut sm = SourceMap::new();
    let file = sm.add_file("t.ya", src.to_string());
    let tokens = Lexer::new(src).tokenize().expect("the lexer does not panic");
    char_literal_value(src, tokens[0].span, file)
}

#[test]
fn a_char_literal_is_one_value_token() {
    let tokens = lex("'A'");
    assert_eq!(tokens[0].kind, TokenKind::Char);
    assert_eq!(char_value("'A'"), Ok(65));
}

#[test]
fn char_escapes_are_decoded() {
    assert_eq!(char_value("'\\n'"), Ok(10));
    assert_eq!(char_value("'\\x41'"), Ok(65));
}

#[test]
fn an_empty_char_literal_is_reported() {
    assert!(first_error("''").message.contains("one byte"));
}

#[test]
fn a_multi_byte_literal_is_reported() {
    assert!(first_error("'AB'").message.contains("one byte"));
    assert!(first_error("'é'").message.contains("one byte"));
}

#[test]
fn a_hex_escape_past_ff_is_reported() {
    assert!(first_error("'\\x1FF'").message.contains("one byte"));
}
```

Use the file's existing helpers (`lex`, `first_error`); if their names differ, follow the local convention.

- [ ] **Step 2: Run them and verify they fail**

```powershell
cargo test -p yashy-lexer --lib a_char_literal_is_one_value_token
```

Expected: FAIL — `'` is currently `unexpected character`.

- [ ] **Step 3: Implement the scanner**

- Add `TokenKind::Char` and its `Naming::Class("a character literal")`.
- Add `scan_char` beside `scan_string`: the opening `'`, then exactly one byte or one escape (`\n \t \r \0 \' \\ \xHH`), then `'`. A missing close, an empty body, a body longer than one byte, a newline, or a multi-byte UTF-8 character is a diagnostic pointing at the offending byte. Reuse `decode_escape`; add `\x` (two hex digits) there if the string lexer does not already support it. (It does not need to: strings keep their current escapes, and `\x` may be char-only.)
- Add `pub fn char_literal_value(src: &str, span: Span, file: FileId) -> Result<u8, Diagnostic>` in `token.rs`, the exact analogue of `literal_as_i64`.
- In `exprs.rs`, `TokenKind::Char` produces `Expr::Literal(Literal::Char(byte), span)`.
- Add `Char(u8)` to `ast.rs::Literal`.

- [ ] **Step 4: Run the tests**

```powershell
cargo test -p yashy-lexer --lib
cargo test -p yashy-parser --test parse
```

Expected: PASS. The parser's `Literal` round-trip test still compiles; add `Char('A')` to its cases.

- [ ] **Step 5: Commit**

```powershell
git add -A
git commit -m "feat: lex single-byte char literals"
git push origin main
```

---

### Task 3.2: Type and lower `char`

**Files:**
- Modify: `crates/yashy-sema/src/ty.rs` (`TypeKind::Char`, `Display`)
- Modify: `crates/yashy-sema/src/ownership.rs` (`linearity_of`)
- Modify: `crates/yashy-sema/src/check_expr.rs` (`resolve_type_in`, `has_equality`, ordering, literal checking, `mangled_type_name`, `type_to_expr`)
- Modify: `crates/yashy-sema/src/const_eval.rs` (`ConstValue::Char`)
- Modify: `crates/yashy-codegen/src/{ir.rs,expr_gen.rs}` (`repr_type`, literal, comparison predicate)
- Test: `crates/yashy-sema/tests/check.rs`, `crates/yashy-codegen/tests/golden/char.ya` + `.expected.ll`

**Interfaces:**
- Produces: `char` resolves; a `char` literal has type `char`; `==`, `!=`, `<`, `<=`, `>`, `>=` are defined between two `char`s and use **unsigned** byte order; arithmetic and mixed `char`/`int` comparisons are errors. `repr_type(char) == "i8"`.
- Consumes: Task 3.1's `Literal::Char`.

- [ ] **Step 1: Write the failing tests**

In `check.rs`:

```rust
#[test]
fn a_char_has_its_own_type() {
    let expr = expr_of_statement("fn f() { const c = 'A'; }", "f", 0);
    assert_eq!(expr_ty(&expr).kind, TypeKind::Char);
}

#[test]
fn chars_compare_with_each_other_only() {
    let (ok, _, _) = check_source("fn f() { if 'A' < 'B' { } if 'A' == 'A' { } }");
    assert!(ok.is_ok(), "chars compare with chars: {ok:?}");
    assert_error("fn f() { if 'A' == 65 { } }", "cannot compare `char` with `int`");
    assert_error("fn f() { const n = 'A' + 1; }", "no arithmetic on `char`");
}
```

The exact diagnostic wording is chosen in Step 3; keep the two assertions in sync with it.

Codegen: add `crates/yashy-codegen/tests/golden/char.ya`:

```yashy
fn main() {
    let low = '\x00';
    let high = '\xFF';
    if low < high { print_str("a"); }
    if high > 'A' { print_str("b"); }
}
```

`let`, not `const`: `const_eval` folds a constant comparison, and a folded `true` would leave no `icmp` in the golden to review. The two bindings keep the comparison in the IR. Phase 4 retires `print_str`, so this file is migrated again there (Task 4.4). The point is the two comparisons: `low < high` must be `icmp ult`, `high > 'A'` must be `icmp ugt`. Add the matching golden test in `crates/yashy-codegen/tests/codegen.rs` following the `arena.rs` pattern (including the `UPDATE_GOLDEN` support).

- [ ] **Step 2: Run them and verify they fail**

```powershell
cargo test -p yashy-sema --test check a_char_has_its_own_type
```

Expected: FAIL — `TypeKind::Char` does not exist / `cannot find type \`char\``.

- [ ] **Step 3: Implement the type**

- `ty.rs`: add `Char` after `Bool`; `Display` writes `char`.
- `ownership.rs`: `linearity_of(Char)` is `Copy`.
- `check_expr.rs`: `"char" => Some(TypeKind::Char)` in *both* modes — a C signature spells a byte `i8`, and `from_c_spelling` has no `char` row to shadow; `mangled_type_name` writes `char`; the literal-checking arm maps `Literal::Char(byte)` to `prim(TypeKind::Char)`; `has_equality` includes `Char`; the ordering check accepts two `Char`s and rejects `Char`/`I64` with `` cannot compare `char` with `int` ``; binary arithmetic rejects `Char` with `` no arithmetic on `char`: convert with `to_int()` ``.
- `const_eval.rs`: add `ConstValue::Char(u8)`, its `ty()`, its `to_literal()`, and equality/ordering.
- `ir.rs`: `repr_type(Char) = "i8"`; the scalar layout is one byte (the same arm as `CInt(8)`).
- `expr_gen.rs`: `Literal::Char(byte)` emits `i8 <byte>`; the comparison lowering picks `icmp ult/ule/ugt/uge` for `Char` (signed predicates stay for `I64`). This is the line the `\xFF` test exists for.
- After adding the variant, `cargo check --workspace --all-targets` names every non-exhaustive match — `mangled_type_name` among them. Handle each one; do not add `_ => unreachable!()`.

- [ ] **Step 4: Run the tests and pin the golden**

```powershell
cargo test -p yashy-sema --test check a_char_has_its_own_type
cargo test -p yashy-sema --test check chars_compare_with_each_other_only
$env:UPDATE_GOLDEN = "1"; cargo test -p yashy-codegen char
Remove-Item Env:\UPDATE_GOLDEN
cargo test -p yashy-codegen char
```

Expected: PASS. Read the generated `char.expected.ll` before committing: the two comparisons must use `icmp ugt`/`icmp ult`.

- [ ] **Step 5: Commit**

```powershell
git add -A
git commit -m "feat: add the char type with unsigned comparison"
git push origin main
```

---

### Task 3.3: Convert between `char`, `int`, `string`, and a string's bytes

**Files:**
- Modify: `crates/yashy-sema/src/check_expr.rs` (scalar method resolution: `to_int`, `to_char`, `to_string`)
- Modify: `crates/yashy-codegen/src/strings.rs` (`str_from_char` lowering)
- Modify: `crates/yashy-runtime/{runtime.c,include/yashy_runtime.h,src/lib.rs}` (`yashy_str_from_char`, `yashy_char_from_int`)
- Test: `check.rs`, `crates/yashy-driver/tests/programs/chars.ya`, `e2e.rs`

**Interfaces:**
- Produces: `c.to_int() -> int`, `n.to_char() -> char` (checked, exits non-zero outside 0–255), `c.to_string() -> string`.
- Consumes: Task 3.2. `byte_at` is **not** touched here: changing its type would break five programs in this commit, so Task 3.5 owns that change together with the migration that keeps the tree green.

- [ ] **Step 1: Write the failing tests**

In `check.rs`:

```rust
#[test]
fn scalar_conversions_have_one_spelling() {
    let (ok, _, _) = check_source("fn f(c: char, n: int) { const a = c.to_int(); const b = n.to_char(); const s = c.to_string(); }");
    assert!(ok.is_ok(), "conversions: {ok:?}");
    assert_error("fn f(c: char) { const n = c as int; }", "use `to_int()`");
    assert_error("fn f(n: int) { const c = n as char; }", "use `to_char()`");
    assert_error("fn f(c: char) { const s = c + 1; }", "no arithmetic on `char`");
}
```

`chars.ya` (written with phase-3 primitives; Task 4.4 migrates the calls):

```yashy
fn main() {
    const ch = 'e';
    print_str(ch.to_string());
    print_str("\n");
    print_i64(ch.to_int());
    print_str("\n");
    print_i64('A'.to_int());
    print_str("\n");
    print_str(65.to_char().to_string());
    print_str("\n");
    print_str('A'.to_string());
    print_str("\n");
}
```

Expected stdout: `e\n101\n65\nA\nA\n`. The e2e test is `chars_convert_and_print`. (`65.to_char()` is an integer literal followed by a method call, not a float: `scan_number` only consumes `.` as a decimal point when a digit follows it.)

- [ ] **Step 2: Run and verify failure**

```powershell
cargo test -p yashy-sema --test check scalar_conversions_have_one_spelling
```

Expected: FAIL — `no method \`to_int\` on \`char\``.

- [ ] **Step 3: Implement the methods**

- Add `fn check_scalar_method(&mut self, receiver: HirExpr, method: &str, args: &[Expr], span: Span) -> Option<HirExpr>` and call it from both method paths:
  - `check_builtin_method` after the binding is resolved (pass a `HirExpr::Ident` of the binding and its type);
  - the `Expr::MethodCall` arm before its "needs a binding" error.
- `char.to_int` → `HirExpr::Cast { expr, ty: int, span }` (the backend already zero-extends `i8`).
- `int.to_char` → a call to the internal builtin `char_from_int`, lowered in codegen to `yashy_char_from_int(i64) -> i8`, which reports `value N is not a character (0-255)` through the runtime's panic path and exits non-zero.
- `char.to_string` → a call to `str_from_char`, lowered to `yashy_str_from_char(i8) -> i8*`.
- `as char` / `as int` from the other type: report `` use `to_char()` `` / `` use `to_int()` `` in the cast checker.
- `byte_at` is untouched in this task; Task 3.5 changes its result type and migrates its callers in one commit.
- Runtime: `yashy_str_from_char(unsigned char)` builds a one-byte owned string; `yashy_char_from_int` checks the range.

- [ ] **Step 4: Run the tests**

```powershell
cargo test -p yashy-sema --test check scalar_conversions_have_one_spelling
cargo test -p yashy-driver --test e2e chars_convert_and_print
cargo test -p yashy-runtime
```

Expected: PASS. `byte_at` still returns `int`, so every existing program still checks; Task 3.5 changes that and the five callers together.

- [ ] **Step 5: Commit**

```powershell
git add -A
git commit -m "feat: convert between char, int, string and string bytes"
git push origin main
```

---

### Task 3.4: Print and interpolate a `char`

**Files:**
- Modify: `crates/yashy-sema/src/check_expr.rs` (`as_string`)
- Modify: `crates/yashy-sema/src/check_expr/debug.rs` (`debug_expr` char arm)
- Modify: `crates/yashy-codegen/src/expr_gen.rs` (`str_from_char` builtin)
- Test: `check.rs`, `crates/yashy-driver/tests/programs/chars.ya`, `e2e.rs`

**Interfaces:**
- Produces: `${c}` writes the character; `${c:?}` writes `'A'` (quoted, with the same escapes a string uses). `print`/`println` inherit both when Phase 4 lands.
- Consumes: 3.2, 3.3.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn a_char_interpolates_as_itself() {
    let (result, _, _) = check_source("fn f(c: char) { const s = \"${c}\"; }");
    assert!(result.is_ok(), "{result:?}");
}
```

In `chars.ya` add `print_str("${'B'}")` (its own line, with a following `print_str("\n")`) and `print_str("${'\\n':?}")`, and assert the e2e output.

- [ ] **Step 2: Run and verify failure**

```powershell
cargo test -p yashy-sema --test check a_char_interpolates_as_itself
```

Expected: FAIL — `cannot interpolate a \`char\` … only \`string\`, \`int\`, \`float\` and \`bool\``.

- [ ] **Step 3: Implement**

- `as_string`: `TypeKind::Char => "str_from_char"`; update the error message's list of printable scalars to include `char`.
- `debug.rs`'s kind dispatch (near line 57, beside `TypeKind::Str`) gains `TypeKind::Char => runtime_call("str_debug_char", …)`, with a new runtime function that renders `'A'` / `'\n'` / `'\''` / `'\\'` / `'\xHH'`.
- Update `debug.rs`'s module comment: the display form now has five scalars.

- [ ] **Step 4: Run the tests**

```powershell
cargo test -p yashy-sema --test check a_char_interpolates_as_itself
cargo test -p yashy-driver --test e2e chars_convert_and_print
```

Expected: PASS.

- [ ] **Step 5: Commit**

```sh
git add -A
git commit -m "feat: print and interpolate chars"
git push origin main
```

---

### Task 3.5: Make `byte_at` a `char` and migrate its callers

The type change and the migration are one commit: `byte_at` returning `char` breaks five programs, and leaving them broken for a task would violate the green rule.

**Files:**
- Modify: `crates/yashy-sema/src/check_expr.rs` (`check_string_method`: `byte_at` → `Char`)
- Modify: `crates/yashy-codegen/src/strings.rs` (`gen_str_byte_at` types the result `i8`)
- Modify: `crates/yashy-runtime/runtime.c` (`yashy_str_byte_at` returns `unsigned char`)
- Modify: `benchmarks/features/array.ya` (line 112)
- Modify: `crates/yashy-driver/tests/programs/json_keys.ya` (line 19)
- Modify: `crates/yashy-driver/tests/programs/string_methods.ya` (line 24)
- Modify: `crates/yashy-driver/tests/programs/wc_words.ya` (line 17)
- Modify: `crates/yashy-driver/tests/programs/chars.ya` (the out-of-range row)
- Modify: `crates/yashy-driver/tests/e2e.rs` (`chars_convert_and_print`'s expected stdout)
- Modify: `crates/yashy-stdlib/yashy/string.ya` (comments only)
- Modify: `docs/tutorial/08-strings.md` (types and examples)
- Test: `crates/yashy-sema/tests/check.rs`

**Interfaces:**
- Produces: `s.byte_at(i) -> char`, `'\0'` for an index past the end. Every caller compiles unchanged in behaviour.
- Consumes: 3.3.

- [ ] **Step 1: Write the failing test**

In `check.rs`:

```rust
#[test]
fn a_string_byte_is_a_char() {
    let expr = expr_of_statement("fn f(s: string) { const b = s.byte_at(0); }", "f", 0);
    assert_eq!(expr_ty(&expr).kind, TypeKind::Char);
}
```

- [ ] **Step 2: Run it and verify it fails**

```powershell
cargo test -p yashy-sema --test check a_string_byte_is_a_char
```

Expected: FAIL — the expression is `int`.

- [ ] **Step 3: Implement the change and migrate in the same commit**

- `check_expr.rs`: `check_string_method`'s `byte_at` result type becomes `Char`.
- `strings.rs`: `gen_str_byte_at` types its call `i8`; the runtime returns `unsigned char`, so 0 stays 0.
- `runtime.c`: `yashy_str_byte_at` returns `unsigned char` (the body is unchanged; only the declared return type moves).
- Apply the conversions:
  - `if text.byte_at(2) != 79` → `if text.byte_at(2) != 'O'`.
  - `if line.byte_at(index) == 34` → `if line.byte_at(index) == '"'`.
  - `print_i64(s.byte_at(2))` → `print_i64(s.byte_at(2).to_int())`.
  - `var byte = line.byte_at(index);` → `let byte = line.byte_at(index).to_int();` and keep the numeric comparisons below it.
- Extend `chars.ya` with the byte rows and update its expected stdout in `e2e.rs`:

```yashy
const s = "hello";
print_str(s.byte_at(1).to_string());   // e
print_str("\n");
print_i64(s.byte_at(1).to_int());      // 101
print_str("\n");
print_i64(s.byte_at(99).to_int());     // 0
print_str("\n");
```

- [ ] **Step 4: Update the tutorial's table and example**

`docs/tutorial/08-strings.md`: `byte_at` is `char`, out of range is `'\0'`; the example prints a byte with `print_i64(s.byte_at(2).to_int())` until Phase 4 adds `println` (then it becomes `println(s.byte_at(2))`). Keep every `// =>` line truthful.

- [ ] **Step 5: Run the suite**

```powershell
cargo test --workspace --no-fail-fast
```

Expected: green; the migrated programs' stdout is unchanged (the `'O'`/`'"'` comparisons are the same locale-free byte comparisons).

- [ ] **Step 6: Commit**

```powershell
git add -A
git commit -m "refactor: make byte_at return a char and migrate its callers"
git push origin main
```

---

## Phase 4 — Printing

### Task 4.1: `print` and `println` replace the three print builtins

**Files:**
- Modify: `crates/yashy-sema/src/check_expr.rs` (builtin table, call checking, `as_string`)
- Modify: `crates/yashy-codegen/src/expr_gen.rs` (`print_line_str` lowering)
- Modify: `crates/yashy-runtime/{runtime.c,include/yashy_runtime.h,src/lib.rs}` (`yashy_print_line_str`)
- Test: `check.rs`, `e2e.rs` (the `print_all.ya` proof program is created in 4.4)

**Interfaces:**
- Produces: `print(x)` (one argument, any printable type), `println(x)`, `println()`; the three old names produce a migration diagnostic. `print_str` survives only as the internal lowering name.
- Consumes: 3.4.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn print_takes_one_value_and_returns_nothing() {
    let (result, _, _) = check_source("fn main() { print(1); println(\"x\"); println(); }");
    assert!(result.is_ok(), "{result:?}");
}

#[test]
fn print_takes_exactly_one_value() {
    assert_error("fn main() { print(); }", "`print` takes one value");
    assert_error("fn main() { println(1, 2); }", "`println` takes one value");
}

#[test]
fn print_str_names_its_replacement() {
    assert_error("fn main() { print_str(\"x\"); }", "`print_str` was replaced: use `println` (or `print`)");
    assert_error("fn main() { print_i64(1); }", "`print_i64` was replaced");
    assert_error("fn main() { print_f64(1.0); }", "`print_f64` was replaced");
}
```

- [ ] **Step 2: Run and verify failure**

```powershell
cargo test -p yashy-sema --test check print_takes_one_value_and_returns_nothing
```

Expected: FAIL — `cannot find function \`print\``.

- [ ] **Step 3: Implement the call form**

- Delete the `print_str`, `print_i64`, `print_f64` entries from the builtin table.
- In the `Expr::Call` arm, before the unknown-function path, special-case `callee == ["print"]` and `["println"]`:
  - `print(x)` → `HirExpr::Call { callee: ["print_str"], args: [as_string(x)], ty: Unit }`.
  - `println(x)` → `HirExpr::Call { callee: ["print_line_str"], args: [as_string(x)], ty: Unit }`.
  - `println()` → `HirExpr::Call { callee: ["print_line_str"], args: [""], ty: Unit }`.
  - Any other arity → `` `print` takes one value `` / `` `println` takes one value ``.
- Add the three removed names to the migration table consulted before the unknown-function diagnostic, with the exact message from the spec.
- Codegen: `"print_line_str" =>` lowers to `call void @yashy_print_line_str(i8* …)`, releasing an owned temporary like `print_str` does.
- Runtime: `yashy_print_line_str(const yashy_str *s)` calls the same binary-mode setup, `fputs` the bytes, then `fputc('\n', stdout)`.

- [ ] **Step 4: Migrate the corpus, the tutorial, and the goldens**

Deleting the three names makes every `.ya` that calls them fail to check, so the migration rides in this commit. Rule, applied in order:
1. `print_str(<string literal ending in `\n`>)` → `println(<same literal without the final `\n`>)`.
2. `print_str(<x>)` where `x` is not a literal → `print(x)`.
3. `print_i64(x)` / `print_f64(x)` → `print(x)`.

```powershell
Get-ChildItem -Recurse -Filter *.ya |
  Where-Object { $_.FullName -notmatch '\\target\\' } |
  ForEach-Object {
    $path = $_.FullName; $text = Get-Content -Raw -LiteralPath $path
    $next = $text -replace 'print_str\((\"[^\"\r\n]*)\\n\"\)', 'println($1")'
    $next = $next -replace 'print_(i64|f64)\(', 'print('
    $next = $next -replace 'print_str\(', 'print('
    if ($next -ne $text) { Set-Content -LiteralPath $path -Value $next -NoNewline -Encoding utf8NoBOM }
  }
```

The same three rules over the tutorial's blocks, then let its harness judge the `// =>` lines. The regex only catches literals on one line; anything it misses fails the e2e or tutorial comparison, which is the oracle:

```powershell
cargo test -p yashy-driver --test e2e 2>&1 | Tee-Object "$env:TEMP\print-migrate.log"
cargo test -p yashy-driver --test tutorial
```

For each failing program, compare its actual stdout with the expected string in `e2e.rs`. A missing newline means a `print_str(msg)` whose `msg` already ended in `\n` at run time: change it to `println(msg)` only if `msg` does not contain the newline, otherwise leave `print(msg)`. Do not edit an expected stdout to match a wrong program.

Finally regenerate the goldens; their print calls move even though their output does not:

```powershell
$env:UPDATE_GOLDEN = "1"; cargo test -p yashy-codegen
Remove-Item Env:\UPDATE_GOLDEN
git diff -- crates/yashy-codegen/tests/golden
```

Expected: every diff is a print-lowering change — a `call void @yashy_print_str` whose constant ends in `\n` becomes `call void @yashy_print_line_str` with the newline removed from the constant — plus the migrated source text. Any other instruction moving is a defect to chase before committing.

- [ ] **Step 5: Full gate**

```powershell
cargo check --workspace --all-targets
cargo test --workspace --no-fail-fast
```

Expected: green.

- [ ] **Step 6: Commit**

```powershell
git add -A
git commit -m "feat: add print and println, retire the print builtins, migrate the corpus"
git push origin main
```

---

### Task 4.2: Every printable type prints

**Files:**
- Modify: `crates/yashy-sema/src/check_expr/debug.rs` (`has_debug_form`, a print-specific error)
- Modify: `crates/yashy-sema/src/check_expr.rs` (`as_string` delegates composites)
- Test: `check.rs`

**Interfaces:**
- Produces: `as_string` returns a string for every type that has a debug form (str, char, int, float, bool, struct, enum, tuple, fixed array, slice, view, `Option`, `Result`). A map, channel, mutex, pointer or function value is a diagnostic naming the type.
- Consumes: 4.1. The debug formatter already supports slices, views and `Result` (`check_expr/debug.rs`), so no new shape is written.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn every_aggregate_prints() {
    let (result, _, _) = check_source(
        "struct P { x: int } enum E { A, B(int) } \
         fn main() { \
           const p = P { x: 1 }; println(p); \
           println(E.A); println((1, true)); \
           const xs = [1, 2, 3]; println(xs); println(xs[0..1]); \
           const o: Option[int] = Some(1); println(o); \
           const r: Result[int, string] = Ok(1); println(r); \
         }",
    );
    assert!(result.is_ok(), "{result:?}");
}

#[test]
fn a_map_has_no_text_form() {
    assert_error(
        "fn main() { const m: map[string]int = map_new(); println(m); }",
        "cannot print a `map[string]int`",
    );
}
```

- [ ] **Step 2: Run and verify failure**

```powershell
cargo test -p yashy-sema --test check every_aggregate_prints
```

Expected: FAIL — `cannot interpolate a \`P\` …`.

- [ ] **Step 3: Implement**

- In `debug.rs`, add `pub(super) fn has_debug_form(kind: &TypeKind) -> bool` matching the same set `debug_expr` answers.
- In `as_string`'s `_` arm: if `has_debug_form(&ty.kind)` → `self.debug_string(expr, span)`; else emit `` cannot print a `{ty}`: {why} `` where `why` is "a map's order is not stable", "a channel has no readable state", "a mutex has no readable state", "a pointer has no value to show", or "a function has no text form".
- Keep `debug_string`'s own message for `${x:?}`.

- [ ] **Step 4: Run the tests**

```powershell
cargo test -p yashy-sema --test check every_aggregate_prints
cargo test -p yashy-sema --test check a_map_has_no_text_form
```

Expected: PASS.

- [ ] **Step 5: Commit**

```sh
git add -A
git commit -m "feat: print every type that has a debug form"
git push origin main
```

---

### Task 4.3: `${x}` uses the print rules

**Files:**
- Modify: `crates/yashy-sema/src/check_expr.rs` (`as_string` now covers composites; interpolation inherits it)
- Test: `crates/yashy-sema/tests/check.rs` (update the two `cannot interpolate` tests)

**Interfaces:**
- Produces: `${p}` for a struct prints its debug shape; `${p:?}` is unchanged. A pointer/map in `${…}` still errors, with the print message.
- Consumes: 4.2.

- [ ] **Step 1: Update the tests first**

`interpolating_a_type_with_no_text_form_is_an_error` currently asserts a struct cannot be interpolated. Replace it with:

```rust
#[test]
fn a_struct_interpolates_with_its_debug_shape() {
    let (result, _, _) = check_source("struct P { x: int } fn f(p: P) { const line = \"${p}\"; }");
    assert!(result.is_ok(), "{result:?}");
}

#[test]
fn a_pointer_still_has_no_text_form() {
    assert_error("fn f() { let p = alloc(1); const line = \"${p}\"; }", "cannot print a `*mut int`");
}
```

- [ ] **Step 2: Run and verify the old behaviour**

```powershell
cargo test -p yashy-sema --test check a_struct_interpolates_with_its_debug_shape
```

Expected: PASS once 4.2's `as_string` change is in place; FAIL before it.

- [ ] **Step 3: Adjust the error wording**

The pointer/map interpolation now says "cannot print", not "cannot interpolate". Update every test needle that quotes the old phrase (`check.rs` lines ~906, ~912) and the comment in `debug.rs` that calls Display scalar-only.

- [ ] **Step 4: Run the tests**

```powershell
cargo test -p yashy-sema --test check
```

Expected: PASS.

- [ ] **Step 5: Commit**

```sh
git add -A
git commit -m "feat: interpolate aggregates with the print form"
git push origin main
```

---

### Task 4.4: Prove the print table end to end

The corpus and its goldens were migrated in 4.1; this task is the user-facing proof that every printable type prints, plus the tutorial's teaching pass.

**Files:**
- Create: `crates/yashy-driver/tests/programs/print_all.ya`
- Modify: `crates/yashy-driver/tests/e2e.rs` (one new test)
- Modify: `docs/tutorial/{01-hello,02-values,08-strings}.md` (use `println(x)` directly instead of the `.to_int()` workaround)

**Interfaces:**
- Produces: one program that prints one line per row of the spec's print table; the tutorial's print examples use the final surface.
- Consumes: 4.1–4.3.

- [ ] **Step 1: Write the proof program**

`print_all.ya` prints one line per row of the spec's table:

```yashy
// Every printable type, printed once. The output is the spec's print table.
struct Point { x: int, y: int }
enum Shape { Circle(float), Empty }

fn main() {
    println("text");
    println('A');
    println(42);
    println(2.5);
    println(true);
    println(Point { x: 3, y: 4 });
    println(Shape.Circle(2.5));
    println((1, true));
    const xs = [1, 2, 3];
    println(xs);
    println(xs[0..2]);
    const maybe: Option[int] = Some(7);
    println(maybe);
    const missing: Option[int] = None;
    println(missing);
    const answer: Result[int, string] = Ok(7);
    println(answer);
    println("1 + 1 = ${1 + 1}");
    println();
}
```

Expected output, in order: `text`, `A`, `42`, `2.5`, `true`, `Point { x: 3, y: 4 }`, the enum's variant form, `(1, true)`, `[1, 2, 3]`, `[1, 2]`, `Some(7)`, `None`, `Ok(7)`, `1 + 1 = 2`, and one empty line.

The enum line is not invented here: `println(Shape.Circle(2.5))` must print exactly what `"${Shape.Circle(2.5):?}"` printed before this phase. Read `debug.rs`'s `enum_text` (or run the interpolation once) and put that text in the assertion. Everything else is pinned by the formatter's doc comments: `Name { field: value }`, `(a, b)`, `[a, b]`, `Some(x)`, `None`, `Ok(x)`.

- [ ] **Step 2: Add the e2e test**

```rust
#[test]
fn print_and_println_cover_every_printable_type() {
    let (output, stdout) = build_and_run("print_all");
    assert!(output.status.success(), "{stdout}");
    // The `Circle(2.5)` segment is `debug.rs`'s enum form; if the formatter
    // renders a variant differently, change this string to match it -- the
    // formatter is the oracle, not this test.
    assert_eq!(
        stdout,
        "text\nA\n42\n2.5\ntrue\nPoint { x: 3, y: 4 }\nCircle(2.5)\n(1, true)\n\
         [1, 2, 3]\n[1, 2]\nSome(7)\nNone\nOk(7)\n1 + 1 = 2\n\n"
    );
}
```

- [ ] **Step 3: Run it**

```powershell
cargo test -p yashy-driver --test e2e print_and_println_cover_every_printable_type
```

Expected: PASS. A failure here is a print-rule defect in 4.1–4.3, not a test to adjust (the single exception is the enum's exact text, which Step 1 told you to copy from the formatter).

- [ ] **Step 4: Finish the tutorial's print examples**

Replace the phase-3 workarounds the migration left behind — `print(c.to_int())`, `print(s.byte_at(2).to_int())` and the like — with the direct form now that it exists (`println(c)`, `println(s.byte_at(2))`), keeping every `// =>` line truthful:

```powershell
cargo test -p yashy-driver --test tutorial
```

Expected: PASS.

- [ ] **Step 5: Full gate**

```powershell
cargo check --workspace --all-targets
cargo test --workspace --no-fail-fast
```

Expected: green.

- [ ] **Step 6: Commit**

```powershell
git add -A
git commit -m "test: prove the print table end to end"
git push origin main
```

---

## Phase 5 — Simplifications

### Task 5.1: Remove `var`

**Files:**
- Modify: `crates/yashy-parser/src/{stmts.rs,ast.rs}` (drop `Stmt::Var`, `BindingKeyword::Var`)
- Modify: `crates/yashy-sema/src/check_expr.rs` (drop the `Stmt::Var` arm)
- Modify: `crates/yashy-fmt/src/**`, `crates/yashy-lsp/src/**` (any keyword list or `Stmt::Var` match)
- Modify: every `*.ya` that uses `var` (about 79 files)
- Test: `crates/yashy-parser/tests/parse.rs`, `crates/yashy-driver/tests/e2e.rs`

**Interfaces:**
- Produces: `let` is the only reassignable binding; `var` is a parse diagnostic at the keyword, with a caret. `Mutability::Var` and `HirStmt::Var` stay — they are internal and `debug.rs` builds them.
- Consumes: the lexer keeps the `var` keyword on purpose: a removed keyword that still lexes as one can be reported by name, while deleting it would turn `var x = 1;` into a confusing "expected a statement" about an identifier.
- Note: `check_source` in `crates/yashy-sema/tests/check.rs` panics on a parse error, so this test belongs in the parser's suite, not the checker's.

- [ ] **Step 1: Write the failing test**

In `crates/yashy-parser/tests/parse.rs`:

```rust
#[test]
fn var_names_its_replacement() {
    let src = "fn f() { var x = 1; }";
    let mut sm = SourceMap::new();
    let file = sm.add_file("t.ya", src.to_string());
    let tokens = Lexer::new(src).tokenize().expect("the lexer does not panic");
    let mut parser = Parser::new(tokens, &sm, file);
    let error = parser.parse_program().expect_err("`var` must be refused");
    assert_eq!(error.message, "`var` was removed: use `let`");
}
```

- [ ] **Step 2: Run and verify failure**

```powershell
cargo test -p yashy-parser --test parse var_names_its_replacement
```

Expected: FAIL — `var` still parses.

- [ ] **Step 3: Implement**

- `stmts.rs`: `TokenKind::Var => Err(self.error(kw.span, "`var` was removed: use `let`"))` in `parse_statement`; delete `parse_var`.
- `ast.rs`: delete `Stmt::Var` and the `Var` variant of `BindingKeyword` (keep `Let`).
- `check_expr.rs`: delete the `Stmt::Var` arm; the `BindingKeyword` match keeps only `Let`.
- Follow the compiler through `yashy-fmt` and `yashy-lsp` for the removed variant.
- Keep `TokenKind::Var` and the `"var"` keyword mapping, with a comment explaining why a removed keyword is still lexed.

- [ ] **Step 4: Migrate the corpus**

```powershell
Get-ChildItem -Recurse -Filter *.ya | ForEach-Object {
  $path = $_.FullName; $text = Get-Content -Raw -LiteralPath $path
  $next = $text -replace '(?m)^(\s*)var\s+','$1let '
  if ($next -ne $text) { Set-Content -LiteralPath $path -Value $next -NoNewline -Encoding utf8NoBOM }
}
```

This only rewrites a `var` that begins a line after indentation. A `var` inside a one-line block (`if x { var y = 1; }`) is missed; the suite names it, and it is fixed by hand.

- [ ] **Step 5: Prove the diagnostic carries a caret, at the CLI**

In `crates/yashy-driver/tests/e2e.rs`, modelled on `a_type_error_produces_a_caret_diagnostic_and_exit_1`:

```rust
#[test]
fn var_is_reported_with_a_caret() {
    let dir = work_dir("var-removed");
    let source = dir.join("bad.ya");
    fs::write(&source, "fn main() {\n    var x = 1;\n}\n").expect("write bad.ya");

    let compile = Command::new(yashy())
        .arg("build")
        .arg(&source)
        .output()
        .expect("run the yashy CLI");

    assert_eq!(compile.status.code(), Some(1), "`var` is a compile error");
    let stderr = stderr_of(&compile);
    assert!(stderr.contains("`var` was removed: use `let`"), "{stderr}");
    assert!(
        stderr.lines().any(|line| line.trim_start().starts_with('^')),
        "expected a caret under `var`:\n{stderr}"
    );
}
```

- [ ] **Step 6: Run the tests**

```powershell
cargo test -p yashy-parser --test parse var_names_its_replacement
cargo test -p yashy-driver --test e2e var_is_reported_with_a_caret
cargo test --workspace --no-fail-fast
Get-ChildItem -Recurse -Filter *.ya | Select-String -Pattern '\bvar\s+\w'
```

Expected: PASS, and no output from the last command. The suite is the proof that every migrated `var` still behaves as a `let`.

- [ ] **Step 7: Commit**

```sh
git add -A
git commit -m "feat: remove var in favour of let"
git push origin main
```

---

### Task 5.2: One `.free()`

**Files:**
- Modify: `crates/yashy-sema/src/check_expr.rs` (`builtin_method_name` pointer arm, `check_string_method`, the old-name table)
- Create: `crates/yashy-driver/tests/programs/free_methods.ya`
- Test: `check.rs`, `e2e.rs`

**Interfaces:**
- Produces: `p.free()` on a `*mut T`, `xs.free()` on `[]T`, `m.free()` on a map, `c.free()` on a channel, `m.free()` on a mutex. `s.free()` on a `string` is a diagnostic. `dealloc(p)` points at `p.free()`.
- Consumes: the existing `free` methods for slices/maps/channels/mutexes; only the pointer is new.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn a_pointer_frees_with_the_dot() {
    let (result, _, _) = check_source("fn f() { let p = alloc(1); p.free(); }");
    assert!(result.is_ok(), "{result:?}");
}

#[test]
fn a_string_has_nothing_to_free() {
    assert_error("fn f(s: string) { s.free(); }", "a `string` releases itself");
}

#[test]
fn dealloc_names_the_method() {
    assert_error("fn f() { let p = alloc(1); dealloc(p); }", "use `.free()`");
}
```

- [ ] **Step 2: Run and verify failure**

```powershell
cargo test -p yashy-sema --test check a_pointer_frees_with_the_dot
```

Expected: FAIL — `no method \`free\` on \`*mut int\``.

- [ ] **Step 3: Implement**

- `builtin_method_name`: `TypeKind::Ptr(_) => match method { "free" => Some("dealloc"), _ => None }`.
- `check_string_method`: `"free"` → error `` a `string` releases itself: there is nothing to free ``.
- Old-name table: `"dealloc" => Some(("free", 1))`, and the rendered message must read `` `dealloc` was replaced: use `.free()` `` — the table cannot name the receiver, so the wording stays receiver-neutral (the test asserts exactly that).
- No runtime change: `dealloc` already lowers to `yashy_free`.

- [ ] **Step 4: Add the e2e program and migrate**

`free_methods.ya` allocates, frees a slice, a map and a pointer, and prints a marker after each. Add `e2e.rs::free_is_one_method`. Migrate the nine `dealloc(` calls across five files: `crates/yashy-codegen/tests/golden/memory.ya` (2), `crates/yashy-driver/tests/programs/buffer.ya` (2), `programs/defer.ya` (1), `programs/memory.ya` (3), `crates/yashy-stdlib/yashy/alloc.ya` (1). The golden's expected IR is unchanged — the method lowers to the same `yashy_free` call.

- [ ] **Step 5: Run the tests**

```powershell
cargo test -p yashy-sema --test check a_pointer_frees_with_the_dot
cargo test -p yashy-sema --test check a_string_has_nothing_to_free
cargo test -p yashy-sema --test check dealloc_names_the_method
cargo test -p yashy-driver --test e2e free_is_one_method
```

Expected: PASS.

- [ ] **Step 6: Commit**

```sh
git add -A
git commit -m "feat: release pointers with the dot free method"
git push origin main
```

---

### Task 5.3: The view message

**Files:**
- Modify: `crates/yashy-sema/src/check_expr.rs` (`TypeExpr::View` arm)
- Modify: `docs/tutorial/09-arrays.md`
- Test: `check.rs`

**Interfaces:**
- Produces: `[T]` is refused with `` a view cannot be named: bind it with `let` and use it in place ``. `a[start..end]` keeps working and keeps being inferrable.
- Consumes: nothing. The refusal already exists; this task aligns the wording with the spec and proves the range still works.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn a_view_cannot_be_named() {
    assert_error("fn f(xs: []int) { const v: [int] = xs[0..1]; }", "a view cannot be named");
}

#[test]
fn a_range_is_still_inferred() {
    let (result, _, _) = check_source("fn f(xs: []int) { let v = xs[0..1]; println(v.len()); }");
    assert!(result.is_ok(), "{result:?}");
}
```

- [ ] **Step 2: Run and verify**

```powershell
cargo test -p yashy-sema --test check a_view_cannot_be_named
```

Expected: PASS (the refusal exists) with the old message — the assertion fails on the wording.

- [ ] **Step 3: Change the message and the tutorial**

Replace the arm's message with the spec's sentence. In `09-arrays.md`, keep the explanation that `a[start..end]` borrows, delete any implication that `[T]` is a writable type, and keep the `// =>` lines.

- [ ] **Step 4: Run the tests**

```powershell
cargo test -p yashy-sema --test check a_view_cannot_be_named
cargo test -p yashy-sema --test check a_range_is_still_inferred
cargo test -p yashy-driver --test tutorial
```

Expected: PASS.

- [ ] **Step 5: Commit**

```sh
git add -A
git commit -m "docs: state the view rule in one sentence"
git push origin main
```

---

## Phase 6 — Docs, tutorial, benchmarks

### Task 6.1: Teach the new surface in the tutorial

**Files:**
- Modify: `docs/tutorial/01-hello.md` (`println`; the `.ya` extension)
- Modify: `docs/tutorial/02-values.md` (`int`, `float`, `void`, `char`, `let`, no `var`)
- Modify: `docs/tutorial/07-memory.md` (`.free()`)
- Modify: `docs/tutorial/08-strings.md` (`byte_at` returns a `char`)
- Modify: `docs/tutorial/09-arrays.md` (two spellings; views inferred)
- Test: `crates/yashy-driver/tests/tutorial.rs`

**Interfaces:**
- Produces: every chapter teaches only spellings that exist; every block compiles and prints its `// =>` lines.
- Consumes: phases 2–5.

- [ ] **Step 1: Add the two chapters' missing sections**

- `02-values.md`: a table of the six scalar spellings with widths; a `char` subsection (literal, escapes, `to_int`/`to_char`, `to_string`, `byte_at`); a `void` paragraph (`fn f()` and `fn f() -> void` are the same; `let x: void` is refused).
- `01-hello.md`: `print` vs `println`, `println()` for a blank line, and `${…}`.
- `07-memory.md`: one `.free()` table (slice/map/pointer; string refuses).
- `09-arrays.md`: `[T; N]` and `[]T`; `a[start..end]` is inferred, never named.

- [ ] **Step 2: Run the tutorial harness**

```powershell
cargo test -p yashy-driver --test tutorial
```

Expected: PASS. Every block compiles; every `// =>` line matches.

- [ ] **Step 3: Commit**

```sh
git add -A
git commit -m "docs: teach the Yashy surface in the tutorial"
git push origin main
```

---

### Task 6.2: Update the README and the performance notes

**Files:**
- Modify: `README.md`
- Modify: `docs/performance-improvements.md`

**Interfaces:**
- Produces: the front page describes `int`/`float`/`char`/`void`, `print`/`println`, `let`, `.free()`, and the two array spellings. The status line no longer lists `var` or `print_i64`.
- Consumes: the whole language.

- [ ] **Step 1: Rewrite the surface paragraphs**

Replace every `i64`/`f64` with `int`/`float`, delete `var`, replace the `print_*` mentions with `print`/`println`, and add one paragraph for `char`. Keep the "Formerly Otter" note at the top.

- [ ] **Step 2: Check the docs for dead spellings**

```powershell
git grep -n -E "\b(var|i64|f64|print_str|print_i64|print_f64|dealloc|free_slice|map_free)\b" -- README.md docs/tutorial docs/performance-improvements.md
```

Expected: no output (a mention of a removed spelling is allowed only where the text explicitly says it was removed; if so, keep it and note why).

- [ ] **Step 3: Commit**

```sh
git add -A
git commit -m "docs: describe the final surface in the README"
git push origin main
```

---

### Task 6.3: Benchmark the new features

**Files:**
- Create: `benchmarks/features/char.ya`
- Create: `benchmarks/features/print.ya`
- Modify: `benchmarks/run-features.ps1` (two new rows)
- Modify: `benchmarks/README.md` (the two rows and the prose)
- Test: the scripts themselves

**Interfaces:**
- Produces: a measured cost for `char` comparison and for `println` throughput, built at `-O2` like every other row.
- Consumes: phases 3–4.

- [ ] **Step 1: Write the two programs**

- `char.ya`: build a 1 KiB `pattern` of printable bytes, take `pattern.repeat(1024)` for a 1 MiB string, then scan it with `s.byte_at(i)`, counting the bytes in `'a'..='z'`. This measures the `char` load, the two unsigned comparisons and the loop.
- `print.ya`: write 200,000 lines with `println(i)` to a redirected file. This measures `print_line_str` and the `str_from_*` conversion. The runner must redirect stdout to `$null`/a file and report bytes written, so the row is not measuring terminal speed.

- [ ] **Step 2: Add the rows and run**

```powershell
cargo build --release --workspace
pwsh benchmarks/run-features.ps1 -Runs 3 -Passes 1
```

Expected: both new rows produce a time and are not `BLOCKED`. If a binary is blocked, retry; a blocked row is not a result.

- [ ] **Step 3: Record the numbers**

Put the measured milliseconds in `benchmarks/README.md`'s table with the date and the machine, and one sentence saying what each row proves. Do not copy a number from another row.

- [ ] **Step 4: Commit**

```sh
git add -A
git commit -m "bench: measure char comparison and println throughput"
git push origin main
```

---

### Task 6.4: Final gate

- [ ] **Step 1: The name is gone**

```powershell
git grep -n -i otter -- ':!docs/superpowers' ':!.superpowers'
```

Expected: only the README's "Formerly Otter" note.

- [ ] **Step 2: No old spellings in the corpus**

```powershell
Get-ChildItem -Recurse -Filter *.ya | Select-String -Pattern '\b(var|i64|f64)\b|print_str|print_i64|print_f64|dealloc\(|free_slice|map_free'
```

Expected: no output.

- [ ] **Step 3: The workspace gate**

```powershell
cargo check --workspace --all-targets
cargo test --workspace --no-fail-fast 2>&1 | Tee-Object "$env:TEMP\final-tests.log"
Select-String -Path "$env:TEMP\final-tests.log" -Pattern 'test result:'
```

Expected: no warnings; every suite `ok.`; the total is the Task 1.1 baseline plus the tests added by phases 2–5. No `BLOCKED` launch counted as a pass.

- [ ] **Step 4: The tutorial**

```powershell
cargo test -p yashy-driver --test tutorial
```

Expected: PASS.

- [ ] **Step 5: Commit the final fixes**

```sh
git add -A
git commit -m "test: close the Yashy rename gate"
git push origin main
```

---

## Recording deviations

Each task's "As built" paragraph is written into this file when the task lands — not later. It records only what differs from the plan: a different file, a renamed helper, a decision the plan left open (for example the exact `char` debug rendering), or a test that had to change its expectation. A task with no deviation says "As built: as planned."

Two deviations are already known and must be recorded:

1. **Phase 2.2** — the spec originally claimed `unit` was a removed spelling. The source spells the no-value type `()`; the spec was corrected on 2026-10-23 before this plan was written. The plan follows the corrected spec.
2. **Phase 4.2** — the spec originally said slices and `Result` had to be added to the debug formatter. `check_expr/debug.rs` already covers both, so the task is a delegation in `as_string`, not new formatting code.
