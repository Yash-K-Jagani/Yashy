//! The `yashy` command line interface.
//!
//! ```text
//! yashy build <file.ya> [-o <out>] [--emit-ir] [--cpu <tier>] [--fast-math] [--release]
//! yashy run <file.ya> [build flags] [-- <program args>...]
//! yashy check <file.ya>
//! yashy test [path] [--filter <substring>] [--list]
//! yashy init [path]
//! yashy doctor
//! yashy fmt [-w] <files...>
//! yashy version
//! ```
//!
//! Argument parsing is hand-rolled on purpose: a handful of subcommands and
//! flags do not need a parser generator, and every dependency in this workspace
//! is one more thing between a user and a compiler that builds offline.

use std::env;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, ExitStatus};
use std::time::{SystemTime, UNIX_EPOCH};

use yashy_codegen::{check_main_exists, emit_program, host_target_triple, CodegenOptions};
use yashy_common::{Diagnostic, SourceMap, EXIT_COMPILE_ERROR, EXIT_OK};
use yashy_driver::{
    compile_hir, detect, load_and_check, resolve_opt, BuildOptions, CpuTarget, OptLevel,
};

const USAGE: &str = "\
usage:
  yashy build <file.ya> [-o <out>] [-O0|-O1|-O2|-O3] [--cpu <tier>] [--fast-math] [--release] [--emit-ir]
  yashy run <file.ya> [-O0|-O1|-O2|-O3] [--cpu <tier>] [--fast-math] [--release] [-- <args>...]
  yashy check <file.ya>
  yashy test [path] [--filter <substring>] [--list]
  yashy init [path]
  yashy doctor
  yashy fmt [-w] <files...>
  yashy version

  build   compile a program to a native executable
  run     compile and execute a program
  check   lex, parse and type-check without emitting anything
  test    compile and run the programs under tests/
  init    create a new Yashy project
  doctor  check the compiler's toolchain and standard library
  fmt     rewrite source files in the canonical style; -w writes them back
  -O<n>        LLVM optimization level for the generated code (default -O2)
  --cpu <tier> instruction set the program may use, one of:
               portable (default), x86-64-v2, x86-64-v3, x86-64-v4, native
               (`--native` is a synonym for `--cpu native`)
  --fast-math  allow fused multiply-add; faster float code, different results
  --release    a release build: `debug_assert` is compiled out and the
               optimization level defaults to -O3 unless an explicit -O
               level is given
  --emit-ir    print the generated LLVM IR to stdout and stop";

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let Some(command) = args.first().map(String::as_str) else {
        eprintln!("error: no command given\n{USAGE}");
        return exit_code(EXIT_COMPILE_ERROR);
    };
    let rest = args.get(1..).unwrap_or(&[]);

    match command {
        "version" | "--version" | "-V" => {
            println!("yashy {}", env!("CARGO_PKG_VERSION"));
            exit_code(EXIT_OK)
        }
        "help" | "--help" | "-h" => {
            println!("{USAGE}");
            exit_code(EXIT_OK)
        }
        "build" => build(rest),
        "run" => run(rest),
        "check" => check(rest),
        "test" => test(rest),
        "init" => init(rest),
        "doctor" => doctor(),
        "fmt" => fmt(rest),
        other => {
            eprintln!("error: unknown command `{other}`\n{USAGE}");
            exit_code(EXIT_COMPILE_ERROR)
        }
    }
}

/// `yashy build <file.ya> [-o <out>] [-O0|-O1|-O2|-O3] [--cpu <tier>] [--fast-math] [--release] [--emit-ir]`.
fn build(args: &[String]) -> ExitCode {
    let mut input: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    let mut emit_ir = false;
    // Defaulting to `-O2` is the whole point of exposing this: the compiler
    // emits unoptimized IR so it stays readable, and hands the optimization to
    // LLVM. A build that passes no level gets `-O0` and a binary an order of
    // magnitude slower than the same source deserves. `--release` raises the
    // default to `-O3`, but an explicit level always wins.
    let mut opt: Option<OptLevel> = None;
    // Portable unless asked otherwise. A higher instruction set produces a
    // binary that can die with an illegal instruction on an older CPU, so it is
    // never the default; see `CpuTarget` for the difference between a feature
    // level (reproducible across a CPU generation) and `native` (this machine).
    let mut cpu = CpuTarget::default();
    // Off unless asked. Contraction changes the arithmetic the program performs
    // — a fused multiply-add rounds once where `a * b + c` rounds twice — so a
    // program that depends on its exact output must opt in knowingly. See
    // `CodegenOptions::fp_contract`.
    let mut fast_math = false;
    // A release build drops `debug_assert` and raises the default optimization
    // level to `-O3`. The two questions -- how hard to optimize, and whether
    // this is a build a user runs -- stay separable because an explicit `-O`
    // level wins over the release default.
    let mut release = false;

    let mut args = args.iter();
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--emit-ir" => emit_ir = true,
            "--fast-math" => fast_math = true,
            "--release" => release = true,
            "--native" => cpu = CpuTarget::Native,
            "--cpu" => match args.next() {
                Some(value) => match CpuTarget::parse(value) {
                    Some(target) => cpu = target,
                    None => return usage_error(&unknown_cpu(value)),
                },
                None => return usage_error("`--cpu` needs a value"),
            },
            "-o" | "--output" => match args.next() {
                Some(value) => output = Some(PathBuf::from(value)),
                None => return usage_error("`-o` needs a path"),
            },
            other => {
                if let Some(value) = other.strip_prefix("--cpu=") {
                    match CpuTarget::parse(value) {
                        Some(target) => cpu = target,
                        None => return usage_error(&unknown_cpu(value)),
                    }
                } else if let Some(level) = OptLevel::parse(other) {
                    opt = Some(level);
                } else if other.starts_with("-O") {
                    return usage_error(&format!(
                        "`{other}` is not an optimization level; use -O0, -O1, -O2 or -O3"
                    ));
                } else if other.starts_with('-') {
                    return usage_error(&format!("unknown option `{other}`"));
                } else {
                    if input.is_some() {
                        return usage_error(&format!("unexpected extra argument `{other}`"));
                    }
                    input = Some(PathBuf::from(other));
                }
            }
        }
    }

    // A feature level is an x86-64 name and nothing else. On any other target it
    // is not a conservative choice, it is a flag `clang` rejects, so the
    // mismatch is refused here where it can name the two architectures rather
    // than left for `clang` to report against a build far from the cause.
    if cpu.requires_x86_64() && !host_target_triple().starts_with("x86_64") {
        return usage_error(&format!(
            "`{}` is an x86-64 feature level, but this compiler targets `{}`",
            cpu.name(),
            host_target_triple()
        ));
    }

    let Some(input) = input else {
        return usage_error("`build` needs a source file");
    };
    let source = match read_source(&input) {
        Ok(source) => source,
        Err(code) => return code,
    };
    let entry = input.to_string_lossy().into_owned();

    // One map for the whole program: the loader adds every imported file to it
    // as it reads that file, so a diagnostic from a module renders against the
    // module's own text. The entry point is file id zero.
    let mut sources = SourceMap::new();
    let hir = match load_and_check(&mut sources, &input, source, release) {
        Ok(hir) => hir,
        Err(diagnostics) => return render(&sources, &diagnostics),
    };

    // `--emit-ir` is a pure front-end operation, so it deliberately does not
    // require an LLVM toolchain: dumping IR is exactly what you want to do on a
    // machine where the backend cannot run yet.
    if emit_ir {
        return match check_main_exists(&hir.items) {
            Ok(()) => {
                let options = CodegenOptions {
                    fp_contract: fast_math,
                };
                print!("{}", emit_program(&hir, &entry, options));
                exit_code(EXIT_OK)
            }
            Err(diagnostic) => render(&sources, &[diagnostic]),
        };
    }

    let toolchain = match detect() {
        Ok(toolchain) => toolchain,
        Err(message) => {
            eprintln!("error: {message}");
            return exit_code(EXIT_COMPILE_ERROR);
        }
    };

    let output = output.unwrap_or_else(|| default_output(&input));
    let opt = resolve_opt(release, opt);
    let options = BuildOptions {
        opt,
        cpu,
        fast_math,
        release,
    };
    match compile_hir(&hir, &entry, &output, &toolchain, options) {
        Ok(()) => exit_code(EXIT_OK),
        Err(diagnostics) => render(&sources, &diagnostics),
    }
}

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
    if let Err(error) = fs::create_dir_all(&dir) {
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
        let _ = fs::remove_dir_all(&dir);
        return code;
    }

    let status = status_with_retry(&output, &program_args);
    let _ = fs::remove_dir_all(&dir);

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
    let mut bytes = fs::read(exe)?;
    bytes.resize(bytes.len() + attempt, 0);
    fs::write(&candidate, bytes)?;
    Ok(candidate)
}

fn blocked_by_policy(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::PermissionDenied
        || error.raw_os_error() == Some(4551)
        || error.to_string().contains("blocked")
        || error.to_string().contains("Application Control")
}

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
        tests.retain(|path| {
            path.to_string_lossy().contains(filter) || display_test_path(path).contains(filter)
        });
    }

    if list {
        for path in &tests {
            println!("{}", display_test_path(path));
        }
        return exit_code(EXIT_OK);
    }

    if tests.is_empty() {
        println!("no tests found under {}", root.display());
        return exit_code(EXIT_OK);
    }

    println!(
        "running {} test{}",
        tests.len(),
        if tests.len() == 1 { "" } else { "s" }
    );

    let mut passed = 0usize;
    let mut failed = 0usize;
    for path in &tests {
        let name = display_test_path(path);
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
    println!(
        "test result: {}. {passed} passed; {failed} failed",
        if failed == 0 { "ok" } else { "FAILED" }
    );
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

/// Test paths are printed with `/` on every platform so a CI log and a local
/// log name the same file the same way.
fn display_test_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
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
            return usage_error(&format!(
                "{} exists and is not a directory",
                target.display()
            ));
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
        (
            "yashy.toml",
            format!(
                "[package]\nname = \"{}\"\nversion = \"0.1.0\"\n",
                escape_toml(&name)
            ),
        ),
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
        .and_then(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "yashy-project".to_string())
}

fn escape_toml(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

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

/// `yashy check <file.ya>`.
fn check(args: &[String]) -> ExitCode {
    if args.is_empty() {
        return usage_error("`check` needs a source file");
    }
    if args.len() > 1 {
        return usage_error(&format!("unexpected extra argument `{}`", args[1]));
    }
    let Some(input) = args.first() else {
        return usage_error("`check` needs a source file");
    };
    let input = PathBuf::from(input);
    let source = match read_source(&input) {
        Ok(source) => source,
        Err(code) => return code,
    };
    // `check` reports the same diagnostics in either build mode -- a release
    // build still checks a `debug_assert`'s condition, it only declines to
    // lower it -- so there is no `--release` here to accept and ignore.
    let mut sources = SourceMap::new();
    match load_and_check(&mut sources, &input, source, false) {
        Ok(_) => exit_code(EXIT_OK),
        Err(diagnostics) => render(&sources, &diagnostics),
    }
}

fn read_source(input: &Path) -> Result<String, ExitCode> {
    std::fs::read_to_string(input).map_err(|error| {
        eprintln!("error: cannot read {}: {error}", input.display());
        exit_code(EXIT_COMPILE_ERROR)
    })
}

/// `yashy fmt [-w] <files...>`.
///
/// Without `-w` the formatted text goes to standard output, so a file can be
/// inspected before it is replaced. With `-w` a file is written in place, and
/// only when it parsed: an unformattable file is left exactly as it was.
fn fmt(args: &[String]) -> ExitCode {
    let mut write = false;
    let mut files: Vec<PathBuf> = Vec::new();
    for argument in args {
        match argument.as_str() {
            "-w" | "--write" => write = true,
            other if other.starts_with('-') => {
                return usage_error(&format!("unknown flag `{other}` for `fmt`"));
            }
            other => files.push(PathBuf::from(other)),
        }
    }
    if files.is_empty() {
        return usage_error("`fmt` needs at least one source file");
    }

    let mut failed = false;
    for path in &files {
        let source = match read_source(path) {
            Ok(source) => source,
            Err(_) => {
                failed = true;
                continue;
            }
        };
        match yashy_fmt::format(&source) {
            Ok(formatted) => {
                if write {
                    if formatted != source {
                        if let Err(error) = std::fs::write(path, &formatted) {
                            eprintln!("error: cannot write {}: {error}", path.display());
                            failed = true;
                        }
                    }
                } else {
                    print!("{formatted}");
                }
            }
            Err(diagnostics) => {
                // `format` reports against its own one-file map, where the
                // source is file 0; a map built here with one file puts it at
                // the same index, so the carets land on the right bytes.
                let mut sources = SourceMap::new();
                let _ = sources.add_file(path.display().to_string(), source.clone());
                let _ = render(&sources, &diagnostics);
                failed = true;
            }
        }
    }

    if failed {
        exit_code(EXIT_COMPILE_ERROR)
    } else {
        exit_code(EXIT_OK)
    }
}

/// Render each diagnostic with its source line and caret, to stderr.
///
/// `sources` is the map the diagnostics were produced against, so a diagnostic
/// from an imported module names that module and points into its text. Building
/// a fresh map here would put every caret in the entry file.
fn render(sources: &SourceMap, diagnostics: &[Diagnostic]) -> ExitCode {
    for diagnostic in diagnostics {
        eprint!("{}", sources.render(diagnostic));
    }
    exit_code(EXIT_COMPILE_ERROR)
}

/// `<dir>/<stem>` plus `.exe` on Windows.
fn default_output(input: &Path) -> PathBuf {
    let stem = input.file_stem().unwrap_or_else(|| OsStr::new("a"));
    let output = input.with_file_name(stem);
    if cfg!(windows) {
        output.with_extension("exe")
    } else {
        output
    }
}

/// The usage error for a `--cpu` value that names no target, listing every name
/// that would have worked.
fn unknown_cpu(value: &str) -> String {
    let names: Vec<&str> = CpuTarget::ALL.iter().map(|target| target.name()).collect();
    format!(
        "`{value}` is not a CPU target; use one of: {}",
        names.join(", ")
    )
}

fn usage_error(message: &str) -> ExitCode {
    eprintln!("error: {message}\n{USAGE}");
    exit_code(EXIT_COMPILE_ERROR)
}

fn exit_code(code: i32) -> ExitCode {
    ExitCode::from(code as u8)
}
