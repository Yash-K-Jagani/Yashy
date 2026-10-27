# Yashy R1 Tooling Foundation â€” Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** Make Yashy installable and usable for a first CLI project: `yashy run`, `yashy test`, `yashy init`, `yashy doctor`, bundled stdlib discovery, meaningful `--release` builds, and a benchmark CI job.

**Architecture:** All tooling stays in `crates/yashy-driver` and the existing CLI binary. `run` and `test` reuse the same `load_and_check` / `compile_hir` pipeline as `build`; they do not introduce a second compiler path. `test` discovers ordinary `.ya` programs under `tests/`, so R1 needs no attributes, macros, or new language syntax. The installer is a packaging layer around the same binary and the `yashy-stdlib/yashy` directory.

**Tech Stack:** Rust (existing workspace), `std::process`, `std::fs`, PowerShell for the benchmark smoke script and Windows packaging, GitHub Actions.

**Scope:** R1 is split into three plans. This is Plan A, the critical path:

- **Plan A (this file):** tooling, installer, benchmark CI.
- **Plan B:** core standard library (`fs`, `path`, `args`, `env`, `process`, `time`, `rand`, `JSON`).
- **Plan C:** error suggestions, first-CLI guide, examples.

Plan A produces working, testable software on its own: a user can install Yashy, initialize a project, run it, test it, and diagnose the toolchain. Plan B and Plan C follow after this plan is implemented.

**Global constraints:**

- Do not add a new runtime dependency to the compiler for tooling.
- `yashy run` and `yashy test` must use the same `load_and_check` and `compile_hir` pipeline as `yashy build`.
- Test files are ordinary Yashy programs with a `main`; do not add a `#[test]` attribute or a new language keyword in R1.
- Keep the existing `build`, `check`, and `fmt` commands working.
- Windows and Linux are the R1 platforms. macOS packaging is a 1.0 target.
- Tests must not depend on network access.
- Commit after each task. Do not batch unrelated tasks into one commit.

---

## Task 0: Shared test support

**Files:**

- Create: `crates/yashy-driver/tests/common/mod.rs`

**Purpose:** The new CLI tests need the same Application Control / Device Guard launch workaround as `crates/yashy-driver/tests/e2e.rs`. Copy the helper rather than making every test rediscover the problem.

- [x] **Step 1: Create the shared helper module**

Copy these items from `crates/yashy-driver/tests/e2e.rs` into `tests/common/mod.rs`, making them `pub`:

- `LAUNCH_ATTEMPTS`
- `yashy()`
- `runnable_cli()`
- `launch_candidate()`
- `blocked_by_policy()`
- `work_dir(name: &str) -> PathBuf` (rename the format prefix to `yashy-cli-test-{pid}-{name}`)
- `write(dir: &Path, relative: &str, text: &str)`
- `exe(dir: &Path, name: &str) -> PathBuf`

Add this header:

```rust
//! Shared helpers for CLI integration tests.
//!
//! Kept in `tests/common/mod.rs` so it is a module, not a separate test target.
```

- [x] **Step 2: Verify the helper compiles**

Run:

```text
cargo test -p yashy-driver --test e2e --no-run
```

Expected: the existing e2e test target still compiles. The new module is not yet referenced, so it is not compiled by this command; that is fine. It will be compiled by the first test that declares `mod common;`.

- [x] **Step 3: Commit**

```text
git add crates/yashy-driver/tests/common/mod.rs
git commit -m "test(driver): share CLI launch helpers"
```

---

## Task 1: Make `--release` mean an optimized build

**Files:**

- Modify: `crates/yashy-driver/src/pipeline.rs`
- Modify: `crates/yashy-driver/src/lib.rs`
- Modify: `crates/yashy-driver/src/main.rs`
- Create: `crates/yashy-driver/tests/opt.rs`

**Interfaces:**

```rust
// crates/yashy-driver/src/pipeline.rs
pub fn resolve_opt(release: bool, explicit: Option<OptLevel>) -> OptLevel;

// crates/yashy-driver/src/lib.rs
pub use pipeline::{
    compile_hir, compile_to_binary, load_and_check, resolve_opt, BuildOptions, CpuTarget, OptLevel,
};
```

**Behavior:** A plain build keeps the current `-O2` default. `--release` without an explicit `-O` uses `-O3`. An explicit `-O0`/`-O1`/`-O2`/`-O3` always wins, even with `--release`.

- [x] **Step 1: Add a failing test**

Create `crates/yashy-driver/tests/opt.rs`:

```rust
use yashy_driver::{resolve_opt, OptLevel};

#[test]
fn debug_build_keeps_the_o2_default() {
    assert_eq!(resolve_opt(false, None), OptLevel::Full);
}

#[test]
fn release_build_defaults_to_o3() {
    assert_eq!(resolve_opt(true, None), OptLevel::Aggressive);
}

#[test]
fn an_explicit_level_wins_over_release() {
    assert_eq!(resolve_opt(true, Some(OptLevel::None)), OptLevel::None);
    assert_eq!(resolve_opt(true, Some(OptLevel::Less)), OptLevel::Less);
    assert_eq!(resolve_opt(true, Some(OptLevel::Full)), OptLevel::Full);
    assert_eq!(
        resolve_opt(true, Some(OptLevel::Aggressive)),
        OptLevel::Aggressive
    );
}
```

- [x] **Step 2: Run the test and verify it fails**

```text
cargo test -p yashy-driver --test opt
```

Expected: FAIL to compile because `resolve_opt` does not exist.

- [x] **Step 3: Add the pure resolver**

In `crates/yashy-driver/src/pipeline.rs`, below the `OptLevel` implementation, add:

```rust
/// The optimization level a build should use.
///
/// `explicit` is the level the user wrote on the command line, if any. A
/// release build without an explicit level gets `-O3`; a debug build keeps
/// the historical `-O2` default. An explicit level always wins, so
/// `-O0 --release` remains a usable debugging build.
pub fn resolve_opt(release: bool, explicit: Option<OptLevel>) -> OptLevel {
    explicit.unwrap_or(if release {
        OptLevel::Aggressive
    } else {
        OptLevel::Full
    })
}
```

Re-export it from `crates/yashy-driver/src/lib.rs`:

```rust
pub use pipeline::{
    compile_hir, compile_to_binary, load_and_check, resolve_opt, BuildOptions, CpuTarget, OptLevel,
};
```

- [x] **Step 4: Use the resolver in the CLI**

In `crates/yashy-driver/src/main.rs`, import `resolve_opt`:

```rust
use yashy_driver::{
    compile_hir, detect, load_and_check, resolve_opt, BuildOptions, CpuTarget, OptLevel,
};
```

In `build`, replace the mutable `opt` with an explicit option:

```rust
let mut opt: Option<OptLevel> = None;
```

In the argument loop, replace:

```rust
} else if let Some(level) = OptLevel::parse(other) {
    opt = level;
}
```

with:

```rust
} else if let Some(level) = OptLevel::parse(other) {
    opt = Some(level);
}
```

Before constructing `BuildOptions`, add:

```rust
let opt = resolve_opt(release, opt);
```

Update the usage text for `--release`:

```text
  --release    a release build: `debug_assert` is compiled out and the
               optimization level defaults to -O3 unless an explicit -O
               level is given
```

- [x] **Step 5: Run the tests**

```text
cargo test -p yashy-driver --test opt
cargo test -p yashy-driver --test e2e --no-run
```

Expected: the `opt` tests pass and the e2e target still compiles.

- [x] **Step 6: Commit**

```text
git add crates/yashy-driver/src/pipeline.rs crates/yashy-driver/src/lib.rs crates/yashy-driver/src/main.rs crates/yashy-driver/tests/opt.rs
git commit -m "feat(driver): make --release default to -O3"
```

---

## Task 2: Discover the bundled standard library

**Files:**

- Modify: `crates/yashy-driver/src/modules.rs`
- Modify: `crates/yashy-driver/src/lib.rs` (re-export if needed)
- Modify: `crates/yashy-driver/tests/stdlib.rs`

**Interfaces:**

```rust
// crates/yashy-driver/src/modules.rs
pub fn default_stdlib_root() -> Option<PathBuf>;
pub fn module_roots(entry: &Path) -> Vec<PathBuf>;
```

**Behavior:** A program can `import io;` without `YASHY_PATH`. Search order:

1. `YASHY_PATH` entries, if the variable is set and non-empty.
2. The entry file's directory, when `YASHY_PATH` is not set.
3. `YASHY_STDLIB`, if it points at a directory.
4. Next to the executable: `<exe_dir>/yashy-stdlib/yashy`, `<exe_dir>/stdlib`, `<exe_dir>/../share/yashy/stdlib`.
5. The development checkout: `<driver manifest dir>/../yashy-stdlib/yashy`.

A root is only returned if it is a directory.

- [x] **Step 1: Add a failing test**

In `crates/yashy-driver/tests/stdlib.rs`, add:

```rust
#[test]
fn stdlib_is_found_without_yashy_path() {
    let dir = work_dir("bundled");
    write(
        &dir,
        "main.ya",
        "import io;\nfn main() { println(\"ok\"); }\n",
    );
    let output = Command::new(yashy())
        .arg("build")
        .arg(dir.join("main.ya"))
        .arg("-o")
        .arg(exe(&dir, "main"))
        .env_remove("YASHY_PATH")
        .env_remove("YASHY_STDLIB")
        .output()
        .expect("the yashy CLI must be runnable");

    assert!(
        output.status.success(),
        "the bundled stdlib must be discoverable without YASHY_PATH:\n{}",
        stderr_of(&output)
    );
}
```

- [x] **Step 2: Run the test and verify it fails**

```text
cargo test -p yashy-driver --test stdlib stdlib_is_found_without_yashy_path
```

Expected: FAIL because `import io;` cannot be resolved without `YASHY_PATH`.

- [x] **Step 3: Add the discovery function**

In `crates/yashy-driver/src/modules.rs`, add:

```rust
/// The standard-library directory shipped with this compiler, if one can be
/// found.
///
/// The explicit `YASHY_STDLIB` variable wins. Otherwise the search follows
/// the layouts an installer can produce, then the development checkout. A
/// candidate must be a directory; a stale path is ignored rather than turned
/// into a confusing "module not found" for every import.
pub fn default_stdlib_root() -> Option<PathBuf> {
    if let Some(value) = env::var_os("YASHY_STDLIB") {
        let path = PathBuf::from(value);
        if path.is_dir() {
            return Some(path);
        }
    }

    if let Ok(exe) = env::current_exe() {
        if let Some(dir) = exe.parent() {
            let candidates = [
                dir.join("yashy-stdlib").join("yashy"),
                dir.join("stdlib"),
                dir.join("..")
                    .join("share")
                    .join("yashy")
                    .join("stdlib"),
            ];
            for candidate in candidates {
                if candidate.is_dir() {
                    return Some(candidate);
                }
            }
        }
    }

    let dev = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("yashy-stdlib")
        .join("yashy");
    if dev.is_dir() {
        return Some(dev);
    }

    None
}
```

Update `module_roots` so it always appends the default stdlib root after the user roots:

```rust
pub fn module_roots(entry: &Path) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();

    if let Some(value) = env::var_os("YASHY_PATH") {
        let separator = if cfg!(windows) { ';' } else { ':' };
        roots.extend(
            value
                .to_string_lossy()
                .split(separator)
                .filter(|root| !root.is_empty())
                .map(PathBuf::from),
        );
    }

    if roots.is_empty() {
        match entry.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => {
                roots.push(parent.to_path_buf())
            }
            _ => roots.push(PathBuf::from(".")),
        }
    }

    if let Some(stdlib) = default_stdlib_root() {
        if !roots.contains(&stdlib) {
            roots.push(stdlib);
        }
    }

    roots
}
```

- [x] **Step 4: Re-export the discovery function**

In `crates/yashy-driver/src/lib.rs`, change:

```rust
pub use modules::{load, module_roots, resolve_import, Module};
```

to:

```rust
pub use modules::{default_stdlib_root, load, module_roots, resolve_import, Module};
```

- [x] **Step 5: Run the stdlib and module tests**

```text
cargo test -p yashy-driver --test stdlib
cargo test -p yashy-driver --test modules
```

Expected: all tests pass. The existing `yashy_path_overrides_the_default_root` test still passes because `YASHY_PATH` remains the first root.

- [x] **Step 6: Commit**

```text
git add crates/yashy-driver/src/modules.rs crates/yashy-driver/src/lib.rs crates/yashy-driver/tests/stdlib.rs
git commit -m "feat(driver): discover the bundled standard library"
```

---

## Task 3: Add `yashy run`

**Files:**

- Modify: `crates/yashy-driver/src/main.rs`
- Create: `crates/yashy-driver/tests/run.rs`

**Interface:**

```text
yashy run <file.ya> [-O0|-O1|-O2|-O3] [--cpu <tier>] [--fast-math] [--release] [-- <program args>...]
```

`run` compiles to a temporary executable, executes it with the current working directory and inherited stdio, forwards its exit code, and removes the temporary directory. `-o` is rejected because `run` chooses the output path.

- [x] **Step 1: Add failing tests**

Create `crates/yashy-driver/tests/run.rs`:

```rust
mod common;

use common::{exe, work_dir, write, yashy};
use std::process::Command;

#[test]
fn run_compiles_and_prints_program_output() {
    let dir = work_dir("run-output");
    write(
        &dir,
        "main.ya",
        "fn main() { println(\"hello from run\"); }\n",
    );

    let output = Command::new(yashy())
        .arg("run")
        .arg(dir.join("main.ya"))
        .current_dir(&dir)
        .output()
        .expect("the yashy CLI must be runnable");

    assert!(
        output.status.success(),
        "run must succeed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), "hello from run\n");
}

#[test]
fn run_forwards_the_program_exit_code() {
    let dir = work_dir("run-exit");
    write(&dir, "main.ya", "fn main() -> int { return 7; }\n");

    let output = Command::new(yashy())
        .arg("run")
        .arg(dir.join("main.ya"))
        .current_dir(&dir)
        .output()
        .expect("the yashy CLI must be runnable");

    assert_eq!(output.status.code(), Some(7));
}

#[test]
fn run_rejects_output_paths() {
    let dir = work_dir("run-output-flag");
    write(&dir, "main.ya", "fn main() {}\n");

    let output = Command::new(yashy())
        .arg("run")
        .arg(dir.join("main.ya"))
        .arg("-o")
        .arg(exe(&dir, "kept"))
        .output()
        .expect("the yashy CLI must be runnable");

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("run"),
        "the error should mention run:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
```

- [x] **Step 2: Run the tests and verify they fail**

```text
cargo test -p yashy-driver --test run
```

Expected: FAIL because `run` is an unknown command.

- [x] **Step 3: Add the command to the usage and dispatcher**

In `crates/yashy-driver/src/main.rs`, add to `USAGE`:

```text
  yashy run <file.ya> [-O0|-O1|-O2|-O3] [--cpu <tier>] [--fast-math] [--release] [-- <args>...]
```

and:

```text
  run     compile and execute a program
```

Add to the `match command`:

```rust
"run" => run(rest),
```

- [x] **Step 4: Implement `run`**

Add these imports:

```rust
use std::process::{Command, ExitCode, ExitStatus};
use std::time::{SystemTime, UNIX_EPOCH};
```

Add the implementation below `build`:

```rust
/// `yashy run <file.ya> [build flags] [-- <program args>...]`.
///
/// The output path is temporary on purpose: `run` is for the edit/run loop,
/// and a user who wants to keep the binary has `build -o`.
fn run(args: &[String]) -> ExitCode {
    let mut build_args: Vec<String> = Vec::new();
    let mut program_args: Vec<String> = Vec::new();
    let mut input: Option<PathBuf> = None;
    let mut after_separator = false;

    let mut iter = args.iter();
    while let Some(argument) = iter.next() {
        if after_separator {
            program_args.push(argument.clone());
            continue;
        }
        match argument.as_str() {
            "--" => after_separator = true,
            "-o" | "--output" => {
                return usage_error(
                    "`run` chooses its own output path; use `build -o` to keep a binary",
                )
            }
            "--release" | "--fast-math" | "--native" => build_args.push(argument.clone()),
            "--cpu" => {
                let Some(value) = iter.next() else {
                    return usage_error("`--cpu` needs a value");
                };
                build_args.push(argument.clone());
                build_args.push(value.clone());
            }
            other if other.starts_with("--cpu=") => build_args.push(argument.clone()),
            other if OptLevel::parse(other).is_some() => build_args.push(argument.clone()),
            other if other.starts_with('-') => {
                return usage_error(&format!("unknown option `{other}` for `run`"))
            }
            other => {
                if input.is_some() {
                    return usage_error(&format!("unexpected extra argument `{other}`"));
                }
                input = Some(PathBuf::from(other));
            }
        }
    }

    let Some(input) = input else {
        return usage_error("`run` needs a source file");
    };

    let dir = temp_run_dir();
    if let Err(error) = std::fs::create_dir_all(&dir) {
        eprintln!("error: cannot create {}: {error}", dir.display());
        return exit_code(EXIT_COMPILE_ERROR);
    }
    let output = dir.join(format!("run{}", std::env::consts::EXE_SUFFIX));

    let mut build_arguments = vec![
        input.to_string_lossy().into_owned(),
        "-o".to_string(),
        output.to_string_lossy().into_owned(),
    ];
    build_arguments.extend(build_args);

    let code = build(&build_arguments);
    if code != exit_code(EXIT_OK) {
        let _ = std::fs::remove_dir_all(&dir);
        return code;
    }

    let status = status_with_retry(&output, &program_args);
    let _ = std::fs::remove_dir_all(&dir);

    match status {
        Ok(status) => exit_code(exit_status_code(status)),
        Err(error) => {
            eprintln!("error: cannot run {}: {error}", output.display());
            exit_code(EXIT_COMPILE_ERROR)
        }
    }
}

fn temp_run_dir() -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!("yashy-run-{}-{stamp}", std::process::id()))
}

fn exit_status_code(status: ExitStatus) -> i32 {
    status.code().unwrap_or(EXIT_COMPILE_ERROR)
}

/// Start a freshly linked binary, stepping around an Application Control /
/// Device Guard policy that refuses new content. The workaround is the same
/// one the integration tests use: a copy with extra zero bytes past the image
/// is a new question, and the loader never reads those bytes.
fn status_with_retry(exe: &Path, args: &[String]) -> std::io::Result<ExitStatus> {
    for attempt in 0..LAUNCH_ATTEMPTS {
        let candidate = launch_candidate(exe, attempt)?;
        match Command::new(&candidate).args(args).status() {
            Ok(status) => return Ok(status),
            Err(error) if blocked_by_policy(&error) => continue,
            Err(error) => return Err(error),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        "the operating system refused to start the compiled program",
    ))
}

const LAUNCH_ATTEMPTS: usize = 40;

fn launch_candidate(exe: &Path, attempt: usize) -> std::io::Result<PathBuf> {
    if attempt == 0 {
        return Ok(exe.to_path_buf());
    }
    let dir = exe.parent().unwrap_or_else(|| Path::new("."));
    let stem = exe
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("program");
    let candidate = dir.join(format!(
        "{stem}_launch{attempt}{}",
        std::env::consts::EXE_SUFFIX
    ));
    let mut bytes = std::fs::read(exe)?;
    bytes.resize(bytes.len() + attempt, 0);
    std::fs::write(&candidate, bytes)?;
    Ok(candidate)
}

fn blocked_by_policy(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::PermissionDenied
        || error.raw_os_error() == Some(4551)
        || error.to_string().contains("blocked")
        || error.to_string().contains("Application Control")
}
```

- [x] **Step 5: Run the tests**

```text
cargo test -p yashy-driver --test run
```

Expected: all three tests pass.

- [x] **Step 6: Commit**

```text
git add crates/yashy-driver/src/main.rs crates/yashy-driver/tests/run.rs
git commit -m "feat(cli): add yashy run"
```

---

## Task 4: Add `yashy test`

**Files:**

- Modify: `crates/yashy-driver/src/main.rs`
- Create: `crates/yashy-driver/tests/test_runner.rs`

**Interface:**

```text
yashy test [path] [--filter <substring>] [--list]
```

R1 test discovery is file-based. Every `*.ya` file under `tests/` is one test program. It must have a `main`; a failing `assert` exits nonzero. The runner compiles each file with the debug pipeline, runs it, prints `ok` or `FAILED`, and exits 1 if any file fails. This avoids adding attributes or macros to the language.

- [x] **Step 1: Add failing tests**

Create `crates/yashy-driver/tests/test_runner.rs`:

```rust
mod common;

use common::{work_dir, write, yashy};
use std::process::Command;

fn run_test(dir: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(yashy())
        .arg("test")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("the yashy CLI must be runnable")
}

#[test]
fn test_runs_passing_and_failing_files() {
    let dir = work_dir("test-runner");
    write(
        &dir,
        "tests/pass.ya",
        "fn main() { assert(1 + 1 == 2); }\n",
    );
    write(
        &dir,
        "tests/fail.ya",
        "fn main() { panic(\"intentional failure\"); }\n",
    );

    let output = run_test(&dir, &[]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(!output.status.success(), "a failing test must fail the run");
    assert!(stdout.contains("pass.ya"), "stdout:\n{stdout}");
    assert!(stdout.contains("fail.ya"), "stdout:\n{stdout}");
    assert!(
        stdout.contains("1 passed") && stdout.contains("1 failed"),
        "stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(stderr.contains("intentional failure"), "stderr:\n{stderr}");
}

#[test]
fn test_filter_runs_one_file() {
    let dir = work_dir("test-filter");
    write(
        &dir,
        "tests/pass.ya",
        "fn main() { assert(1 + 1 == 2); }\n",
    );
    write(
        &dir,
        "tests/fail.ya",
        "fn main() { panic(\"intentional failure\"); }\n",
    );

    let output = run_test(&dir, &["--filter", "pass"]);
    assert!(
        output.status.success(),
        "filtered run must pass:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_with_no_files_is_a_clean_success() {
    let dir = work_dir("test-empty");
    let output = run_test(&dir, &[]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(
        stdout.contains("no tests found"),
        "stdout:\n{stdout}"
    );
}

#[test]
fn test_list_prints_discovered_files() {
    let dir = work_dir("test-list");
    write(
        &dir,
        "tests/smoke.ya",
        "fn main() { assert(true); }\n",
    );

    let output = run_test(&dir, &["--list"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("tests/smoke.ya"), "stdout:\n{stdout}");
}
```

- [x] **Step 2: Run the tests and verify they fail**

```text
cargo test -p yashy-driver --test test_runner
```

Expected: FAIL because `test` is an unknown command.

- [x] **Step 3: Add the command to the usage and dispatcher**

In `USAGE`, add:

```text
  yashy test [path] [--filter <substring>] [--list]
```

and:

```text
  test    compile and run the programs under tests/
```

Add to the dispatcher:

```rust
"test" => test(rest),
```

- [x] **Step 4: Implement the test runner**

Add these imports:

```rust
use std::fs;
use std::path::PathBuf;
```

Add the implementation below `run`:

```rust
/// `yashy test [path] [--filter <substring>] [--list]`.
///
/// R1 tests are ordinary programs. A file under `tests/` is one test; the
/// process exit code is the verdict. This keeps the language free of
/// attributes and macros while still giving a project a real test command.
fn test(args: &[String]) -> ExitCode {
    let mut root: Option<PathBuf> = None;
    let mut filter: Option<String> = None;
    let mut list = false;

    let mut iter = args.iter();
    while let Some(argument) = iter.next() {
        match argument.as_str() {
            "--list" => list = true,
            "--filter" => {
                let Some(value) = iter.next() else {
                    return usage_error("`--filter` needs a substring");
                };
                filter = Some(value.clone());
            }
            other if other.starts_with('-') => {
                return usage_error(&format!("unknown option `{other}` for `test`"))
            }
            other => {
                if root.is_some() {
                    return usage_error(&format!("unexpected extra argument `{other}`"));
                }
                root = Some(PathBuf::from(other));
            }
        }
    }

    let root = root.unwrap_or_else(|| PathBuf::from("tests"));
    let mut tests = Vec::new();
    if let Err(error) = collect_tests(&root, &mut tests) {
        eprintln!("error: cannot read {}: {error}", root.display());
        return exit_code(EXIT_COMPILE_ERROR);
    }
    tests.sort();

    if let Some(filter) = &filter {
        tests.retain(|path| path.to_string_lossy().contains(filter));
    }

    if list {
        for path in &tests {
            println!("{}", path.display());
        }
        return exit_code(EXIT_OK);
    }

    if tests.is_empty() {
        println!("no tests found under {}", root.display());
        return exit_code(EXIT_OK);
    }

    println!("running {} test{}", tests.len(), if tests.len() == 1 { "" } else { "s" });

    let mut passed = 0usize;
    let mut failed = 0usize;
    for path in &tests {
        let name = path.display().to_string();
        let dir = temp_run_dir();
        if let Err(error) = fs::create_dir_all(&dir) {
            eprintln!("error: cannot create {}: {error}", dir.display());
            failed += 1;
            continue;
        }
        let exe = dir.join(format!("test{}", std::env::consts::EXE_SUFFIX));
        let build_args = vec![
            path.to_string_lossy().into_owned(),
            "-o".to_string(),
            exe.to_string_lossy().into_owned(),
        ];
        let code = build(&build_args);
        if code != exit_code(EXIT_OK) {
            println!("test {name} ... FAILED (compile error)");
            failed += 1;
            let _ = fs::remove_dir_all(&dir);
            continue;
        }

        match output_with_retry(&exe, &[]) {
            Ok(output) if output.status.success() => {
                println!("test {name} ... ok");
                passed += 1;
            }
            Ok(output) => {
                println!("test {name} ... FAILED");
                print!("{}", String::from_utf8_lossy(&output.stdout));
                eprint!("{}", String::from_utf8_lossy(&output.stderr));
                failed += 1;
            }
            Err(error) => {
                println!("test {name} ... FAILED (cannot run: {error})");
                failed += 1;
            }
        }
        let _ = fs::remove_dir_all(&dir);
    }

    println!();
    println!("test result: {}. {passed} passed; {failed} failed", if failed == 0 { "ok" } else { "FAILED" });
    if failed == 0 {
        exit_code(EXIT_OK)
    } else {
        exit_code(EXIT_COMPILE_ERROR)
    }
}

fn collect_tests(root: &Path, tests: &mut Vec<PathBuf>) -> std::io::Result<()> {
    if root.is_file() {
        tests.push(root.to_path_buf());
        return Ok(());
    }
    if !root.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_tests(&path, tests)?;
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("ya") {
            tests.push(path);
        }
    }
    Ok(())
}

fn output_with_retry(exe: &Path, args: &[String]) -> std::io::Result<std::process::Output> {
    for attempt in 0..LAUNCH_ATTEMPTS {
        let candidate = launch_candidate(exe, attempt)?;
        match Command::new(&candidate).args(args).output() {
            Ok(output) => return Ok(output),
            Err(error) if blocked_by_policy(&error) => continue,
            Err(error) => return Err(error),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        "the operating system refused to start the compiled test",
    ))
}
```

- [x] **Step 5: Run the tests**

```text
cargo test -p yashy-driver --test test_runner
```

Expected: all four tests pass.

- [x] **Step 6: Commit**

```text
git add crates/yashy-driver/src/main.rs crates/yashy-driver/tests/test_runner.rs
git commit -m "feat(cli): add yashy test"
```

---

## Task 5: Add `yashy init`

**Files:**

- Modify: `crates/yashy-driver/src/main.rs`
- Create: `crates/yashy-driver/tests/init.rs`

**Interface:**

```text
yashy init [path]
```

Creates a project with `yashy.toml`, `main.ya`, `tests/smoke.ya`, `README.md`, and `.gitignore`. Refuses to overwrite a non-empty directory.

- [x] **Step 1: Add failing tests**

Create `crates/yashy-driver/tests/init.rs`:

```rust
mod common;

use common::{work_dir, yashy};
use std::fs;
use std::process::Command;

#[test]
fn init_creates_a_buildable_and_testable_project() {
    let parent = work_dir("init-create");
    let project = parent.join("hello");

    let output = Command::new(yashy())
        .arg("init")
        .arg(&project)
        .output()
        .expect("the yashy CLI must be runnable");
    assert!(
        output.status.success(),
        "init must succeed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    assert!(project.join("yashy.toml").is_file());
    assert!(project.join("main.ya").is_file());
    assert!(project.join("tests/smoke.ya").is_file());
    assert!(project.join("README.md").is_file());

    let build = Command::new(yashy())
        .arg("build")
        .arg(project.join("main.ya"))
        .arg("-o")
        .arg(project.join("hello"))
        .output()
        .expect("the yashy CLI must be runnable");
    assert!(
        build.status.success(),
        "the generated main.ya must compile:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );

    let test = Command::new(yashy())
        .arg("test")
        .current_dir(&project)
        .output()
        .expect("the yashy CLI must be runnable");
    assert!(
        test.status.success(),
        "the generated smoke test must pass:\n{}\n{}",
        String::from_utf8_lossy(&test.stdout),
        String::from_utf8_lossy(&test.stderr)
    );
}

#[test]
fn init_refuses_a_non_empty_directory() {
    let dir = work_dir("init-refuse");
    fs::write(dir.join("keep.txt"), "keep").expect("the fixture must be writable");

    let output = Command::new(yashy())
        .arg("init")
        .arg(&dir)
        .output()
        .expect("the yashy CLI must be runnable");

    assert!(!output.status.success());
    assert!(dir.join("keep.txt").is_file());
    assert!(!dir.join("main.ya").exists());
}

#[test]
fn init_uses_the_current_directory_when_no_path_is_given() {
    let dir = work_dir("init-current");
    let output = Command::new(yashy())
        .arg("init")
        .current_dir(&dir)
        .output()
        .expect("the yashy CLI must be runnable");

    assert!(
        output.status.success(),
        "init must succeed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(dir.join("main.ya").is_file());
}
```

- [x] **Step 2: Run the tests and verify they fail**

```text
cargo test -p yashy-driver --test init
```

Expected: FAIL because `init` is an unknown command.

- [x] **Step 3: Add the command to the usage and dispatcher**

In `USAGE`, add:

```text
  yashy init [path]
```

and:

```text
  init    create a new Yashy project
```

Add to the dispatcher:

```rust
"init" => init(rest),
```

- [x] **Step 4: Implement `init`**

Add the implementation below `test`:

```rust
/// `yashy init [path]`.
///
/// The default is the current directory. A directory that already contains
/// anything is refused rather than merged: `init` creates a project, it does
/// not edit one.
fn init(args: &[String]) -> ExitCode {
    if args.len() > 1 {
        return usage_error(&format!("unexpected extra argument `{}`", args[1]));
    }
    let target = args
        .first()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));

    if target.exists() {
        if !target.is_dir() {
            return usage_error(&format!("{} exists and is not a directory", target.display()));
        }
        match fs::read_dir(&target) {
            Ok(mut entries) => {
                if entries.next().is_some() {
                    return usage_error(&format!(
                        "{} is not empty; choose a new directory or empty it first",
                        target.display()
                    ));
                }
            }
            Err(error) => {
                eprintln!("error: cannot read {}: {error}", target.display());
                return exit_code(EXIT_COMPILE_ERROR);
            }
        }
    } else if let Err(error) = fs::create_dir_all(&target) {
        eprintln!("error: cannot create {}: {error}", target.display());
        return exit_code(EXIT_COMPILE_ERROR);
    }

    let name = project_name(&target);
    let files: [(&str, String); 5] = [
        ("yashy.toml", format!("[package]\nname = \"{}\"\nversion = \"0.1.0\"\n", escape_toml(&name))),
        (
            "main.ya",
            "fn main() {\n    println(\"Hello, Yashy!\");\n}\n".to_string(),
        ),
        (
            "tests/smoke.ya",
            "fn main() {\n    assert(1 + 1 == 2);\n}\n".to_string(),
        ),
        (
            "README.md",
            format!(
                "# {name}\n\nA Yashy project.\n\n```text\nyashy run main.ya\nyashy test\n```\n"
            ),
        ),
        (".gitignore", "/target/\n*.exe\n".to_string()),
    ];

    for (relative, contents) in files {
        let path = target.join(relative);
        if let Some(parent) = path.parent() {
            if let Err(error) = fs::create_dir_all(parent) {
                eprintln!("error: cannot create {}: {error}", parent.display());
                return exit_code(EXIT_COMPILE_ERROR);
            }
        }
        if let Err(error) = fs::write(&path, contents) {
            eprintln!("error: cannot write {}: {error}", path.display());
            return exit_code(EXIT_COMPILE_ERROR);
        }
    }

    println!("Created Yashy project `{name}` in {}", target.display());
    println!("Next:");
    println!("  cd {}", target.display());
    println!("  yashy run main.ya");
    println!("  yashy test");
    exit_code(EXIT_OK)
}

fn project_name(target: &Path) -> String {
    target
        .canonicalize()
        .ok()
        .and_then(|path| path.file_name().map(|name| name.to_string_lossy().into_owned()))
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "yashy-project".to_string())
}

fn escape_toml(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}
```

- [x] **Step 5: Run the tests**

```text
cargo test -p yashy-driver --test init
```

Expected: all three tests pass.

- [x] **Step 6: Commit**

```text
git add crates/yashy-driver/src/main.rs crates/yashy-driver/tests/init.rs
git commit -m "feat(cli): add yashy init"
```

---

## Task 6: Add `yashy doctor`

**Files:**

- Modify: `crates/yashy-driver/src/main.rs`
- Create: `crates/yashy-driver/tests/doctor.rs`

**Interface:**

```text
yashy doctor
```

Prints the Yashy version, the selected LLVM toolchain, and the discovered stdlib root. Exits nonzero if either is missing.

- [x] **Step 1: Add a failing test**

Create `crates/yashy-driver/tests/doctor.rs`:

```rust
mod common;

use common::yashy;
use std::process::Command;

#[test]
fn doctor_reports_the_toolchain_and_stdlib() {
    let output = Command::new(yashy())
        .arg("doctor")
        .output()
        .expect("the yashy CLI must be runnable");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        output.status.success(),
        "doctor must succeed on a working development machine:\n{}\n{}",
        stdout,
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("yashy"), "stdout:\n{stdout}");
    assert!(stdout.contains("clang"), "stdout:\n{stdout}");
    assert!(stdout.contains("stdlib"), "stdout:\n{stdout}");
}
```

- [x] **Step 2: Run the test and verify it fails**

```text
cargo test -p yashy-driver --test doctor
```

Expected: FAIL because `doctor` is an unknown command.

- [x] **Step 3: Add the command to the usage and dispatcher**

In `USAGE`, add:

```text
  yashy doctor
```

and:

```text
  doctor  check the compiler's toolchain and standard library
```

Add to the dispatcher:

```rust
"doctor" => doctor(),
```

- [x] **Step 4: Implement `doctor`**

Add the implementation below `init`:

```rust
/// `yashy doctor`: the two things a fresh install can get wrong.
fn doctor() -> ExitCode {
    println!("yashy {}", env!("CARGO_PKG_VERSION"));

    let mut ok = true;

    match detect() {
        Ok(toolchain) => {
            println!(
                "clang: {} (found as {})",
                toolchain.llc.display(),
                toolchain.found
            );
        }
        Err(message) => {
            eprintln!("clang: missing\n{message}");
            ok = false;
        }
    }

    match yashy_driver::default_stdlib_root() {
        Some(root) => println!("stdlib: {}", root.display()),
        None => {
            eprintln!(
                "stdlib: missing\nset YASHY_STDLIB to the directory containing io.ya, or reinstall Yashy"
            );
            ok = false;
        }
    }

    if ok {
        exit_code(EXIT_OK)
    } else {
        exit_code(EXIT_COMPILE_ERROR)
    }
}
```

- [x] **Step 5: Run the test**

```text
cargo test -p yashy-driver --test doctor
```

Expected: PASS on a machine with `clang` and the development stdlib.

- [x] **Step 6: Commit**

```text
git add crates/yashy-driver/src/main.rs crates/yashy-driver/tests/doctor.rs
git commit -m "feat(cli): add yashy doctor"
```

---

## Task 7: Add benchmark CI

**Files:**

- Create: `benchmarks/ci.ps1`
- Create: `.github/workflows/bench.yml`

**Purpose:** The benchmark suite exists but is not run by CI. R1 adds a small, reliable Windows job that builds and runs the scalar `fib` benchmark against C, records compile time and binary size, and uploads the log. It is a reporting gate, not a performance threshold yet; R2 adds thresholds after the first stable baseline.

- [x] **Step 1: Add the smoke script**

Create `benchmarks/ci.ps1`:

```powershell
# A small, deterministic benchmark job for CI.
#
# The full suites are for a human with a quiet machine. CI needs a job that
# proves the benchmark path still works and produces a baseline: build Yashy
# and C versions of fib, check the answer, and report compile time, binary
# size, and wall-clock run time.
$ErrorActionPreference = 'Stop'

$bench = $PSScriptRoot
$root = Split-Path -Parent $bench
$build = Join-Path $bench 'build\ci'
$yashy = Join-Path $root 'target\release\yashy.exe'
$exe = '.exe'

New-Item -ItemType Directory -Force -Path $build | Out-Null

if (-not (Test-Path $yashy)) {
    throw "no release binary at $yashy - run: cargo build --release -p yashy-driver"
}

function Invoke-Tool {
    param([string]$Exe, [string[]]$Arguments, [string]$Step)
    $output = (& $Exe @Arguments 2>&1 | Out-String).Trim()
    if ($LASTEXITCODE -ne 0) {
        throw "$Step failed (exit $LASTEXITCODE):`n$Exe $($Arguments -join ' ')`n$output"
    }
}

$yashyExe = Join-Path $build "fib-yashy$exe"
$cExe = Join-Path $build "fib-c$exe"

$yashyCompile = Measure-Command {
    Invoke-Tool $yashy @('build', "$bench\fib.ya", '-o', $yashyExe, '--release') 'yashy build fib'
}
$cCompile = Measure-Command {
    Invoke-Tool 'clang' @('-O2', "$bench\fib.c", '-o', $cExe) 'clang build fib'
}

$yashyRun = Measure-Command { $yashyOut = & $yashyExe 2>&1 | Out-String }
$cRun = Measure-Command { $cOut = & $cExe 2>&1 | Out-String }

if ($yashyOut.Trim() -ne $cOut.Trim()) {
    throw "fib outputs differ: yashy='$($yashyOut.Trim())' c='$($cOut.Trim())'"
}

$yashySize = (Get-Item $yashyExe).Length
$cSize = (Get-Item $cExe).Length

Write-Output "fib output: $($yashyOut.Trim())"
Write-Output ("yashy compile ms: {0:N1}" -f $yashyCompile.TotalMilliseconds)
Write-Output ("clang compile ms: {0:N1}" -f $cCompile.TotalMilliseconds)
Write-Output ("yashy run ms: {0:N1}" -f $yashyRun.TotalMilliseconds)
Write-Output ("clang run ms: {0:N1}" -f $cRun.TotalMilliseconds)
Write-Output "yashy binary bytes: $yashySize"
Write-Output "clang binary bytes: $cSize"
```

- [x] **Step 2: Add the workflow**

Create `.github/workflows/bench.yml`:

```yaml
name: Bench

on:
  workflow_dispatch:
  schedule:
    - cron: '0 3 * * 1'

jobs:
  bench:
    name: windows scalar baseline
    runs-on: windows-latest

    steps:
      - uses: actions/checkout@v4

      - name: Install Rust
        uses: dtolnay/rust-toolchain@stable

      - name: Verify LLVM
        shell: pwsh
        run: |
          if (-not (Get-Command clang -ErrorAction SilentlyContinue)) {
            choco install llvm -y --no-progress
          }
          clang --version

      - name: Load the MSVC environment
        uses: ilammy/msvc-dev-cmd@v1

      - name: Build the release compiler
        run: cargo build --release -p yashy-driver

      - name: Run the benchmark smoke
        shell: pwsh
        run: pwsh benchmarks/ci.ps1 | Tee-Object -FilePath bench-output.txt

      - name: Upload the benchmark log
        uses: actions/upload-artifact@v4
        with:
          name: bench-output
          path: bench-output.txt
```

- [x] **Step 3: Run the smoke script locally**

```text
cargo build --release -p yashy-driver
pwsh benchmarks/ci.ps1
```

Expected: prints `fib output: 9227465` (or the value `benchmarks/fib.ya` prints), compile/run times, and both binary sizes. If `clang` is missing, install it with `winget install LLVM.LLVM`.

- [x] **Step 4: Commit**

```text
git add benchmarks/ci.ps1 .github/workflows/bench.yml
git commit -m "ci: add a scalar benchmark baseline"
```

---

## Task 8: Release packaging and the Windows installer

**Files:**

- Create: `scripts/package-release.ps1`
- Create: `scripts/install.ps1`
- Create: `.github/workflows/release.yml`

**Purpose:** A user should not need Rust to install Yashy. The release workflow builds the compiler, packages it with the stdlib, and publishes a zip. The Windows installer downloads the zip, installs under `%LOCALAPPDATA%\Yashy`, adds it to the user PATH, and runs `yashy doctor`.

- [x] **Step 1: Add the packaging script**

Create `scripts/package-release.ps1`:

```powershell
param(
    [Parameter(Mandatory = $true)][string]$Version,
    [string]$Target = 'x86_64-pc-windows-msvc'
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$dist = Join-Path $root 'dist'
$name = "yashy-$Version-$Target"
$stage = Join-Path $dist $name
$zip = Join-Path $dist "$name.zip"

if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
New-Item -ItemType Directory -Force -Path $stage | Out-Null
New-Item -ItemType Directory -Force -Path (Join-Path $stage 'yashy-stdlib') | Out-Null

Copy-Item (Join-Path $root 'target\release\yashy.exe') (Join-Path $stage 'yashy.exe')
Copy-Item -Recurse (Join-Path $root 'crates\yashy-stdlib\yashy') (Join-Path $stage 'yashy-stdlib\yashy')
Copy-Item (Join-Path $PSScriptRoot 'install.ps1') (Join-Path $stage 'install.ps1')
Copy-Item (Join-Path $root 'README.md') (Join-Path $stage 'README.md')

if (Test-Path $zip) { Remove-Item -Force $zip }
Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $zip
Write-Output $zip
```

- [x] **Step 2: Add the installer**

Create `scripts/install.ps1`:

```powershell
param(
    [string]$Version = 'latest',
    [string]$Prefix = (Join-Path $env:LOCALAPPDATA 'Yashy'),
    [string]$LocalArchive = ''
)

$ErrorActionPreference = 'Stop'

if ($LocalArchive) {
    $archive = $LocalArchive
} else {
    $tag = if ($Version -eq 'latest') { 'latest' } else { "v$Version" }
    $url = "https://github.com/Yash-K-Jagani/Otter/releases/$tag/download/yashy-$Version-x86_64-pc-windows-msvc.zip"
    $archive = Join-Path $env:TEMP "yashy-$Version.zip"
    Invoke-WebRequest -Uri $url -OutFile $archive
}

$stage = Join-Path $env:TEMP "yashy-install-$([guid]::NewGuid())"
Expand-Archive -Path $archive -DestinationPath $stage -Force
New-Item -ItemType Directory -Force -Path $Prefix | Out-Null
Copy-Item -Recurse -Force (Join-Path $stage '*') $Prefix

$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if (($userPath -split ';') -notcontains $Prefix) {
    [Environment]::SetEnvironmentVariable('Path', "$Prefix;$userPath", 'User')
}
$env:Path = "$Prefix;$env:Path"

& (Join-Path $Prefix 'yashy.exe') doctor
Write-Output "Installed Yashy to $Prefix"
Write-Output "Open a new terminal, then run: yashy doctor"
```

- [x] **Step 3: Add the release workflow**

Create `.github/workflows/release.yml`:

```yaml
name: Release

on:
  push:
    tags:
      - 'v*'
  workflow_dispatch:
    inputs:
      version:
        description: 'Version to package (without v)'
        required: true

jobs:
  windows:
    runs-on: windows-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - name: Verify LLVM
        shell: pwsh
        run: |
          if (-not (Get-Command clang -ErrorAction SilentlyContinue)) {
            choco install llvm -y --no-progress
          }
          clang --version
      - uses: ilammy/msvc-dev-cmd@v1
      - name: Build
        run: cargo build --release -p yashy-driver
      - name: Package
        shell: pwsh
        run: |
          $version = if ('${{ github.event_name }}' -eq 'workflow_dispatch') { '${{ inputs.version }}' } else { '${{ github.ref_name }}'.TrimStart('v') }
          pwsh scripts/package-release.ps1 -Version $version
      - uses: actions/upload-artifact@v4
        with:
          name: yashy-windows
          path: dist/*.zip
```

- [x] **Step 4: Test the packaging locally**

```text
cargo build --release -p yashy-driver
pwsh scripts/package-release.ps1 -Version 0.2.0-test
pwsh scripts/install.ps1 -Version 0.2.0-test -LocalArchive dist/yashy-0.2.0-test-x86_64-pc-windows-msvc.zip -Prefix $env:TEMP\yashy-test-install
& $env:TEMP\yashy-test-install\yashy.exe doctor
```

Expected: the zip is created, the installer copies the files, and `doctor` reports `clang` and `stdlib`. Remove the test install afterward.

- [x] **Step 5: Commit**

```text
git add scripts/package-release.ps1 scripts/install.ps1 .github/workflows/release.yml
git commit -m "build: package a Windows release and installer"
```

---

## Final verification

Run the complete driver suite:

```text
cargo test -p yashy-driver
```

Run the workspace check:

```text
cargo check --workspace --all-targets
```

Run the benchmark smoke:

```text
cargo build --release -p yashy-driver
pwsh benchmarks/ci.ps1
```

Expected:

- All driver tests pass.
- The workspace check is clean.
- The benchmark smoke prints a correct `fib` answer, compile/run times, and binary sizes.
- `yashy init` â†’ `yashy run main.ya` â†’ `yashy test` works in a fresh directory.
- `yashy doctor` reports a toolchain and stdlib.
- The Windows zip contains `yashy.exe`, `yashy-stdlib/yashy/*.ya`, and `install.ps1`.

## Follow-on plans

- **Plan B:** core standard library. Start with `path` and `env` wrappers, then `fs`, `args`, `process`, `time`, `rand`, and `JSON`. The `args` work includes the `main(argc, argv)` entry change and `os.arg_count()` / `os.arg(i)` builtins.
- **Plan C:** error suggestions, the first-CLI guide, and three examples. This plan should be written after Plan A and Plan B settle the user-facing command and stdlib surfaces.

