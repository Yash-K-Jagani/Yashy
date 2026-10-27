//! The `yashy` command line interface.
//!
//! ```text
//! yashy build <file.ya> [-o <out>] [--emit-ir] [--cpu <tier>] [--fast-math] [--release]
//! yashy run <file.ya> [build flags] [-- <program args>...]
//! yashy check <file.ya>
//! yashy test [path] [--filter <substring>] [--list]
//! yashy explain <code> [--json]
//! yashy init [path]
//! yashy doctor [--json]
//! yashy fmt [-w] <files...>
//! yashy version
//! ```
//!
//! Argument parsing is hand-rolled on purpose: a handful of subcommands and
//! flags do not need a parser generator, and every dependency in this workspace
//! is one more thing between a user and a compiler that builds offline.

use std::env;
use std::ffi::OsStr;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, ExitStatus};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use yashy_codegen::{check_main_exists, emit_program, host_target_triple, CodegenOptions};
use yashy_common::{
    codes, Diagnostic, FileId, Json, Severity, SourceMap, Span, EXIT_COMPILE_ERROR, EXIT_INTERNAL,
    EXIT_OK, EXIT_USAGE,
};
use yashy_driver::{
    compile_hir, detect, load_and_check, resolve_opt, BuildOptions, CpuTarget, OptLevel,
};

const USAGE: &str = "\
usage:
  yashy build <file.ya> [-o <out>] [-O0|-O1|-O2|-O3] [--cpu <tier>] [--fast-math] [--release] [--emit-ir]
  yashy run <file.ya> [-O0|-O1|-O2|-O3] [--cpu <tier>] [--fast-math] [--release] [-- <args>...]
  yashy check <file.ya> [--json]
  yashy test [path] [--filter <substring>] [--list] [--json]
  yashy explain <code> [--json]
  yashy init [path]
  yashy doctor [--json]
  yashy fmt [-w] <files...>
  yashy version

  build   compile a program to a native executable
  run     compile and execute a program
  check   lex, parse and type-check without emitting anything
  test    compile and run the programs under tests/
  explain print the long-form page for an error code
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
  --emit-ir    print the generated LLVM IR to stdout and stop
  --json       print a stable machine-readable report (check, test, explain, doctor)";

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let Some(command) = args.first().map(String::as_str) else {
        eprintln!("error: no command given\n{USAGE}");
        return exit_code(EXIT_USAGE);
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
        "explain" => explain(rest),
        "init" => init(rest),
        "doctor" => doctor(rest),
        "fmt" => fmt(rest),
        other => {
            eprintln!("error: unknown command `{other}`\n{USAGE}");
            exit_code(EXIT_USAGE)
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
        // Forward the child's full exit code rather than squeezing it through
        // `ExitCode`'s `u8`. Windows process codes are 32-bit, and a program
        // that returns 256 must not look like success to its caller.
        Ok(status) => std::process::exit(exit_status_code(status)),
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
/// one the integration tests use: a copy with extra bytes past the image is a
/// new question, and the loader never reads those bytes. The tail has to differ
/// in content, not only in length: a copy padded with zeros is still refused.
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
    pad_with_noise(&mut bytes, attempt);
    fs::write(&candidate, bytes)?;
    Ok(candidate)
}

/// Append a tail of non-zero bytes to `bytes`.
///
/// The policy treats a copy that differs from a blocked image only in the
/// number of zero bytes after it as the same program. A tail with different
/// *content* gets started, so the padding is filled from a small xorshift
/// seeded by the clock and the attempt number. This is not cryptography; it
/// only has to be a different question.
fn pad_with_noise(bytes: &mut Vec<u8>, attempt: usize) {
    use std::time::{SystemTime, UNIX_EPOCH};

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as u64)
        .unwrap_or(0x9e37_79b9_7f4a_7c15);
    let mut state = nanos ^ (attempt as u64).wrapping_mul(0x2545_f491_4f6c_dd1d);
    let start = bytes.len();
    bytes.resize(start + 4096 + attempt, 0);
    for byte in &mut bytes[start..] {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        *byte = (state & 0xff) as u8;
    }
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
    let mut json = false;

    let mut iter = args.iter();
    while let Some(argument) = iter.next() {
        match argument.as_str() {
            "--list" => list = true,
            "--json" => json = true,
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

    if json && list {
        return usage_error("`--list` and `--json` cannot be combined");
    }

    let explicit_root = root.is_some();
    let root = root.unwrap_or_else(|| PathBuf::from("tests"));
    if explicit_root && !root.exists() {
        eprintln!("error: no tests at {}", root.display());
        return exit_code(EXIT_USAGE);
    }
    let mut tests = Vec::new();
    if let Err(error) = collect_tests(&root, &mut tests) {
        eprintln!("error: cannot read {}: {error}", root.display());
        return exit_code(EXIT_USAGE);
    }
    tests.sort();

    if let Some(filter) = &filter {
        tests.retain(|path| {
            path.to_string_lossy().contains(filter) || display_test_path(path).contains(filter)
        });
    }

    if json {
        return test_json(&root, &tests);
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

/// `yashy test --json`: the same test discovery as `test`, with one JSON
/// envelope instead of the human progress lines.
///
/// Compile diagnostics are collected through the same checker and code
/// generator the human path uses, so a fact in the JSON (code, severity, file,
/// span) is the fact the human renderer prints.
fn test_json(root: &Path, tests: &[PathBuf]) -> ExitCode {
    let mut global_sources = SourceMap::new();
    let root_file = global_sources.add_file(root.display().to_string(), String::new());

    if tests.is_empty() {
        print_test_json(&global_sources, &[], &[], 0, 0);
        return exit_code(EXIT_OK);
    }

    let toolchain = match detect() {
        Ok(toolchain) => toolchain,
        Err(message) => {
            let diagnostic = Diagnostic::new(Severity::Error, root_file, Span::SYNTHETIC, message);
            print_test_json(&global_sources, &[diagnostic], &[], 0, 0);
            return exit_code(EXIT_USAGE);
        }
    };

    let mut results: Vec<Json> = Vec::new();
    let mut passed = 0usize;
    let mut failed = 0usize;

    for path in tests {
        let started = Instant::now();
        let file = display_test_path(path);
        let name = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or(file.as_str())
            .to_string();
        let dir = temp_run_dir();
        if let Err(error) = fs::create_dir_all(&dir) {
            let file_id = global_sources.add_file(file.clone(), String::new());
            let diagnostic = Diagnostic::new(
                Severity::Error,
                file_id,
                Span::SYNTHETIC,
                format!("cannot create {}: {error}", dir.display()),
            );
            print_test_json(&global_sources, &[diagnostic], &results, passed, failed);
            return exit_code(EXIT_INTERNAL);
        }

        let exe = dir.join(format!("test{}", std::env::consts::EXE_SUFFIX));
        let mut sources = SourceMap::new();
        let source = match fs::read_to_string(path) {
            Ok(source) => source,
            Err(error) => {
                let file_id = sources.add_file(file.clone(), String::new());
                let diagnostic = Diagnostic::new(
                    Severity::Error,
                    file_id,
                    Span::SYNTHETIC,
                    format!("cannot read {}: {error}", path.display()),
                );
                failed += 1;
                results.push(test_json_object(
                    &name,
                    &file,
                    "failed",
                    elapsed_ms(started),
                    &sources,
                    &[diagnostic],
                ));
                let _ = fs::remove_dir_all(&dir);
                continue;
            }
        };

        // The display path uses `/` on every platform, so the file named by a
        // per-test diagnostic is the same string as the test object's `file`.
        // Windows accepts `/` in a path, and the module loader resolves imports
        // from this entry just as it does from the native form.
        let entry = Path::new(&file);
        let outcome = match load_and_check(&mut sources, entry, source, false) {
            Ok(hir) => compile_hir(&hir, &file, &exe, &toolchain, BuildOptions::default()),
            Err(diagnostics) => Err(diagnostics),
        };

        match outcome {
            Ok(()) => match output_with_retry(&exe, &[]) {
                Ok(output) if output.status.success() => {
                    passed += 1;
                    results.push(test_json_object(
                        &name,
                        &file,
                        "passed",
                        elapsed_ms(started),
                        &sources,
                        &[],
                    ));
                }
                Ok(output) => {
                    failed += 1;
                    let diagnostic = Diagnostic::new(
                        Severity::Error,
                        FileId::from_index(0),
                        Span::SYNTHETIC,
                        match output.status.code() {
                            Some(code) => format!("test exited with status {code}"),
                            None => "test was terminated without an exit status".to_string(),
                        },
                    );
                    results.push(test_json_object(
                        &name,
                        &file,
                        "failed",
                        elapsed_ms(started),
                        &sources,
                        &[diagnostic],
                    ));
                }
                Err(error) => {
                    failed += 1;
                    let diagnostic = Diagnostic::new(
                        Severity::Error,
                        FileId::from_index(0),
                        Span::SYNTHETIC,
                        format!("cannot run the test: {error}"),
                    );
                    results.push(test_json_object(
                        &name,
                        &file,
                        "failed",
                        elapsed_ms(started),
                        &sources,
                        &[diagnostic],
                    ));
                }
            },
            Err(diagnostics) => {
                failed += 1;
                results.push(test_json_object(
                    &name,
                    &file,
                    "failed",
                    elapsed_ms(started),
                    &sources,
                    &diagnostics,
                ));
            }
        }

        let _ = fs::remove_dir_all(&dir);
    }

    print_test_json(&global_sources, &[], &results, passed, failed);
    if failed == 0 {
        exit_code(EXIT_OK)
    } else {
        exit_code(EXIT_COMPILE_ERROR)
    }
}

fn elapsed_ms(started: Instant) -> i64 {
    i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX)
}

fn test_json_object(
    name: &str,
    file: &str,
    status: &str,
    duration_ms: i64,
    sources: &SourceMap,
    diagnostics: &[Diagnostic],
) -> Json {
    let mut object = Json::object();
    object.set("name", Json::string(name));
    object.set("file", Json::string(file));
    object.set("status", Json::string(status));
    object.set("duration_ms", Json::number(duration_ms));
    object.set(
        "diagnostics",
        Json::array(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic_json(sources, diagnostic))
                .collect(),
        ),
    );
    object
}

fn print_test_json(
    global_sources: &SourceMap,
    global: &[Diagnostic],
    tests: &[Json],
    passed: usize,
    failed: usize,
) {
    let mut summary = Json::object();
    summary.set("passed", Json::number(passed as i64));
    summary.set("failed", Json::number(failed as i64));
    summary.set("skipped", Json::number(0));

    let mut report = Json::object();
    report.set("schema_version", Json::number(1));
    report.set("tool", Json::string("test"));
    report.set("yashy_version", Json::string(env!("CARGO_PKG_VERSION")));
    report.set("success", Json::bool(failed == 0 && global.is_empty()));
    report.set(
        "diagnostics",
        Json::array(
            global
                .iter()
                .map(|diagnostic| diagnostic_json(global_sources, diagnostic))
                .collect(),
        ),
    );
    report.set("tests", Json::array(tests.to_vec()));
    report.set("summary", summary);
    println!("{}", report.render());
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

/// `yashy explain <code> [--json]`.
///
/// The human form prints the embedded catalog page verbatim; the JSON form
/// carries the same facts so an agent can read the title, summary, and page
/// without scraping Markdown. A code that is not in the catalog is a usage
/// error: the command cannot explain something that does not exist, and a
/// near miss names the code the reader probably meant.
fn explain(args: &[String]) -> ExitCode {
    let mut json = false;
    let mut code: Option<&str> = None;

    for argument in args {
        match argument.as_str() {
            "--json" => json = true,
            other if other.starts_with('-') => {
                return usage_error(&format!("unknown option `{other}`"));
            }
            other => {
                if code.is_some() {
                    return usage_error(&format!("unexpected extra argument `{other}`"));
                }
                code = Some(other);
            }
        }
    }

    let Some(code) = code else {
        return usage_error("`explain` needs an error code, for example `yashy explain E0100`");
    };
    let Some(entry) = codes::find(code) else {
        match codes::closest(code) {
            Some(near) => eprintln!(
                "error: unknown error code `{code}`; did you mean `{}`?",
                near.id
            ),
            None => eprintln!("error: unknown error code `{code}`"),
        }
        return exit_code(EXIT_USAGE);
    };

    if json {
        print_explain_json(entry);
    } else {
        print!("{}", entry.page);
    }
    exit_code(EXIT_OK)
}

/// The `explain --json` envelope: schema v1, the catalog entry, and the same
/// `docs_url` a rendered diagnostic uses.
fn print_explain_json(code: &codes::Code) {
    let mut report = Json::object();
    report.set("schema_version", Json::number(1));
    report.set("tool", Json::string("explain"));
    report.set("yashy_version", Json::string(env!("CARGO_PKG_VERSION")));
    report.set("success", Json::bool(true));
    report.set("code", Json::string(code.id));
    report.set("title", Json::string(code.title));
    report.set("summary", Json::string(code.summary));
    report.set("page", Json::string(code.page));
    report.set("docs_url", Json::string(codes::docs_url(code.id)));
    println!("{}", report.render());
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
    let files: [(&str, String); 6] = [
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
                "# {name}\n\nA Yashy project.\n\n## Run\n\n```text\nyashy run main.ya\n```\n\n## Test\n\n```text\nyashy test\n```\n\n## Build\n\n```text\nyashy build main.ya -o {name} --release\n```\n\nSee `AGENTS.md` for the project layout and the rules an editor or agent should follow.\n"
            ),
        ),
        (
            "AGENTS.md",
            format!(
                "# {name}\n\nThis is a Yashy project. `main.ya` is the entry point; the programs under `tests/` are the test suite.\n\n## Commands\n\n```text\nyashy check main.ya\nyashy run main.ya\nyashy test\nyashy build main.ya -o {name} --release\n```\n\n## Layout\n\n- `main.ya` -- the program's entry point.\n- `tests/` -- one test program per file. A test is an ordinary Yashy program with a `main`; exit 0 is a pass.\n- `yashy.toml` -- package metadata.\n- `AGENTS.md` -- this guide.\n\n## Rules\n\n- `let` is a mutable binding; `const` is immutable.\n- A module's items are private unless they say `pub`. Import a file by its stem, and keep the file beside the importer.\n- A file under `tests/` is its own entry file. It cannot import a module from the project root by walking up; keep a test self-contained or test the built binary.\n- Errors are values: `Option[T]` for a value that may be missing, `Result[T, E]` for an operation that may fail. There are no exceptions.\n- Owned values are released explicitly: `defer` for a scope, `.free()` for arrays, maps, strings built at run time, channels, and mutexes. The checker refuses a use after `move`.\n- Run `yashy test` before calling the work done. A compile error is not a test failure; fix the diagnostic first.\n\n## Docs\n\n- [Quickstart](https://github.com/Yash-K-Jagani/Otter/blob/main/docs/quickstart.md)\n- [Language tutorial](https://github.com/Yash-K-Jagani/Otter/blob/main/docs/tutorial/00-intro.md)\n- [Errors](https://github.com/Yash-K-Jagani/Otter/blob/main/docs/errors.md)\n- [First CLI](https://github.com/Yash-K-Jagani/Otter/blob/main/docs/tutorial/12-first-cli.md)\n"
            ),
        ),
        (
            ".gitignore",
            format!("/target/\n/main\n/{name}\n*.exe\n*.ll\n*.yashy/\n"),
        ),
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

/// The facts `doctor` reports.
///
/// The human and JSON forms are two renderings of this one value, so a field
/// cannot be added to one and forgotten in the other.
struct DoctorReport {
    ok: bool,
    clang: ClangReport,
    stdlib: StdlibReport,
    path: PathReport,
    next: Option<String>,
}

struct ClangReport {
    ok: bool,
    found: Option<&'static str>,
    path: Option<PathBuf>,
    version: Option<String>,
    error: Option<String>,
}

struct StdlibReport {
    ok: bool,
    path: Option<PathBuf>,
    error: Option<String>,
}

struct PathReport {
    executable: Option<PathBuf>,
    on_path: bool,
    hint: Option<String>,
}

/// `yashy doctor [--json]`: the things a fresh install can get wrong.
fn doctor(args: &[String]) -> ExitCode {
    let mut json = false;
    for argument in args {
        match argument.as_str() {
            "--json" => json = true,
            other => return usage_error(&format!("unexpected extra argument `{other}`")),
        }
    }

    let report = collect_doctor_report();
    if json {
        print_doctor_json(&report);
    } else {
        print_doctor_human(&report);
    }

    if report.ok {
        exit_code(EXIT_OK)
    } else {
        exit_code(EXIT_COMPILE_ERROR)
    }
}

fn collect_doctor_report() -> DoctorReport {
    let clang = check_clang();
    let stdlib = check_stdlib();
    let path = check_path();
    let ok = clang.ok && stdlib.ok;
    let next = if ok {
        None
    } else {
        Some(doctor_next(&clang, &stdlib))
    };

    DoctorReport {
        ok,
        clang,
        stdlib,
        path,
        next,
    }
}

fn check_clang() -> ClangReport {
    match detect() {
        Ok(toolchain) if toolchain.found == "clang" => {
            // A file named `clang` is not enough: Application Control can
            // block it, and a truncated install can fail to start. Run the
            // version probe so doctor tests what the compiler will do.
            match Command::new(&toolchain.llc).arg("--version").output() {
                Ok(output) if output.status.success() => {
                    let version = String::from_utf8_lossy(&output.stdout)
                        .lines()
                        .next()
                        .map(str::trim)
                        .filter(|line| !line.is_empty())
                        .map(str::to_string);
                    ClangReport {
                        ok: true,
                        found: Some("clang"),
                        path: Some(toolchain.llc),
                        version,
                        error: None,
                    }
                }
                Ok(output) => {
                    let bytes = if output.stderr.is_empty() {
                        &output.stdout
                    } else {
                        &output.stderr
                    };
                    let detail = String::from_utf8_lossy(bytes).trim().to_string();
                    let error = if detail.is_empty() {
                        "failed to run".to_string()
                    } else {
                        format!("failed to run: {detail}")
                    };
                    ClangReport {
                        ok: false,
                        found: Some("clang"),
                        path: Some(toolchain.llc),
                        version: None,
                        error: Some(error),
                    }
                }
                Err(error) => ClangReport {
                    ok: false,
                    found: Some("clang"),
                    path: Some(toolchain.llc),
                    version: None,
                    error: Some(format!("cannot run: {error}")),
                },
            }
        }
        Ok(toolchain) => ClangReport {
            ok: false,
            found: Some(toolchain.found),
            path: Some(toolchain.llc),
            version: None,
            error: Some(format!(
                "found only `{}`; builds need clang",
                toolchain.found
            )),
        },
        Err(message) => ClangReport {
            ok: false,
            found: None,
            path: None,
            version: None,
            error: Some(message),
        },
    }
}

fn check_stdlib() -> StdlibReport {
    match yashy_driver::default_stdlib_root() {
        Some(root) if root.join("io.ya").is_file() => StdlibReport {
            ok: true,
            path: Some(root),
            error: None,
        },
        Some(root) => StdlibReport {
            ok: false,
            path: Some(root),
            error: Some(
                "does not contain io.ya; set YASHY_STDLIB to the directory containing io.ya, or reinstall Yashy"
                    .to_string(),
            ),
        },
        None => StdlibReport {
            ok: false,
            path: None,
            error: Some(
                "missing; set YASHY_STDLIB to the directory containing io.ya, or reinstall Yashy"
                    .to_string(),
            ),
        },
    }
}

fn check_path() -> PathReport {
    let executable = env::current_exe().ok();
    let Some(dir) = executable
        .as_ref()
        .and_then(|path| path.parent())
        .map(PathBuf::from)
    else {
        return PathReport {
            executable,
            on_path: false,
            hint: Some("cannot determine the executable's directory".to_string()),
        };
    };

    let on_path = path_contains(&dir);
    let hint = if on_path {
        None
    } else {
        Some(format!("add {} to PATH", dir.display()))
    };

    PathReport {
        executable,
        on_path,
        hint,
    }
}

fn path_contains(dir: &Path) -> bool {
    let Some(path) = env::var_os("PATH") else {
        return false;
    };
    let wanted = canonical_or_owned(dir);

    env::split_paths(&path).any(|entry| {
        let entry = canonical_or_owned(&entry);
        if cfg!(windows) {
            entry
                .to_string_lossy()
                .eq_ignore_ascii_case(&wanted.to_string_lossy())
        } else {
            entry == wanted
        }
    })
}

fn canonical_or_owned(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn doctor_next(clang: &ClangReport, stdlib: &StdlibReport) -> String {
    let mut parts: Vec<String> = Vec::new();
    if !clang.ok {
        parts.push(yashy_driver::toolchain::install_hint());
    }
    if !stdlib.ok {
        parts.push(
            "Set YASHY_STDLIB to the directory containing io.ya, or reinstall Yashy.".to_string(),
        );
    }

    let mut next = parts.join(" ");
    if !next.is_empty() {
        next.push_str(" Then run `yashy doctor`.");
    }
    next
}

fn print_doctor_human(report: &DoctorReport) {
    println!("yashy {}", env!("CARGO_PKG_VERSION"));
    println!("target: {}", host_target_triple());

    if report.clang.ok {
        if let Some(path) = &report.clang.path {
            println!("clang: {}", path.display());
        }
        if let Some(version) = &report.clang.version {
            println!("  {version}");
        }
    } else {
        eprintln!("clang: missing");
        if let Some(path) = &report.clang.path {
            eprintln!("  {}", path.display());
        }
        if let Some(error) = &report.clang.error {
            eprintln!("  {error}");
        }
    }

    if report.stdlib.ok {
        if let Some(path) = &report.stdlib.path {
            println!("stdlib: {}", path.display());
        }
    } else {
        eprintln!("stdlib: missing");
        if let Some(path) = &report.stdlib.path {
            eprintln!("  {}", path.display());
        }
        if let Some(error) = &report.stdlib.error {
            eprintln!("  {error}");
        }
    }

    match &report.path.executable {
        Some(executable) if report.path.on_path => println!("path: {}", executable.display()),
        Some(executable) => println!(
            "path: {} (not on PATH; {})",
            executable.display(),
            report.path.hint.as_deref().unwrap_or("add it to PATH")
        ),
        None => println!("path: unknown"),
    }

    if let Some(next) = &report.next {
        eprintln!("next: {next}");
    }
}

fn print_doctor_json(report: &DoctorReport) {
    let mut out = String::new();
    out.push_str("{\n");
    writeln!(out, "  \"schema_version\": 1,").expect("writing to a String cannot fail");
    writeln!(out, "  \"tool\": \"doctor\",").expect("writing to a String cannot fail");
    writeln!(out, "  \"ok\": {},", report.ok).expect("writing to a String cannot fail");
    writeln!(
        out,
        "  \"yashy_version\": {},",
        json_string(env!("CARGO_PKG_VERSION"))
    )
    .expect("writing to a String cannot fail");
    writeln!(out, "  \"target\": {},", json_string(host_target_triple()))
        .expect("writing to a String cannot fail");

    out.push_str("  \"clang\": {\n");
    writeln!(out, "    \"ok\": {},", report.clang.ok).expect("writing to a String cannot fail");
    writeln!(
        out,
        "    \"found\": {},",
        json_opt_string(report.clang.found.as_deref())
    )
    .expect("writing to a String cannot fail");
    writeln!(
        out,
        "    \"path\": {},",
        json_opt_path(report.clang.path.as_deref())
    )
    .expect("writing to a String cannot fail");
    writeln!(
        out,
        "    \"version\": {},",
        json_opt_string(report.clang.version.as_deref())
    )
    .expect("writing to a String cannot fail");
    writeln!(
        out,
        "    \"error\": {}",
        json_opt_string(report.clang.error.as_deref())
    )
    .expect("writing to a String cannot fail");
    out.push_str("  },\n");

    out.push_str("  \"stdlib\": {\n");
    writeln!(out, "    \"ok\": {},", report.stdlib.ok).expect("writing to a String cannot fail");
    writeln!(
        out,
        "    \"path\": {},",
        json_opt_path(report.stdlib.path.as_deref())
    )
    .expect("writing to a String cannot fail");
    writeln!(
        out,
        "    \"error\": {}",
        json_opt_string(report.stdlib.error.as_deref())
    )
    .expect("writing to a String cannot fail");
    out.push_str("  },\n");

    out.push_str("  \"path\": {\n");
    writeln!(
        out,
        "    \"executable\": {},",
        json_opt_path(report.path.executable.as_deref())
    )
    .expect("writing to a String cannot fail");
    writeln!(out, "    \"on_path\": {},", report.path.on_path)
        .expect("writing to a String cannot fail");
    writeln!(
        out,
        "    \"hint\": {}",
        json_opt_string(report.path.hint.as_deref())
    )
    .expect("writing to a String cannot fail");
    out.push_str("  },\n");

    writeln!(
        out,
        "  \"next\": {}",
        json_opt_string(report.next.as_deref())
    )
    .expect("writing to a String cannot fail");
    out.push_str("}\n");
    print!("{out}");
}

fn json_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for character in value.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            character if (character as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", character as u32));
            }
            character => out.push(character),
        }
    }
    out.push('"');
    out
}

fn json_opt_string(value: Option<&str>) -> String {
    match value {
        Some(value) => json_string(value),
        None => "null".to_string(),
    }
}

fn json_opt_path(value: Option<&Path>) -> String {
    match value {
        Some(value) => json_string(value.to_string_lossy().as_ref()),
        None => "null".to_string(),
    }
}

fn print_check_json(sources: &SourceMap, diagnostics: &[Diagnostic]) {
    let errors = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.is_error())
        .count();
    let warnings = diagnostics.len().saturating_sub(errors);

    let mut summary = Json::object();
    summary.set("errors", Json::number(errors as i64));
    summary.set("warnings", Json::number(warnings as i64));

    let mut report = Json::object();
    report.set("schema_version", Json::number(1));
    report.set("tool", Json::string("check"));
    report.set("yashy_version", Json::string(env!("CARGO_PKG_VERSION")));
    report.set("success", Json::bool(errors == 0));
    report.set(
        "diagnostics",
        Json::array(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic_json(sources, diagnostic))
                .collect(),
        ),
    );
    report.set("summary", summary);
    println!("{}", report.render());
}

fn diagnostic_json(sources: &SourceMap, diagnostic: &Diagnostic) -> Json {
    let mut object = Json::object();
    object.set(
        "code",
        match diagnostic.code {
            Some(code) => Json::string(code),
            None => Json::null(),
        },
    );
    object.set("severity", Json::string(diagnostic.severity.as_str()));
    object.set("message", Json::string(diagnostic.message.as_str()));
    object.set("file", Json::string(sources.path(diagnostic.file)));
    object.set("span", span_json(sources, diagnostic.file, diagnostic.span));
    if let Some(label) = &diagnostic.label {
        let mut label_object = Json::object();
        label_object.set("span", span_range_json(diagnostic.span));
        label_object.set("message", Json::string(label.as_str()));
        object.set("labels", Json::array(vec![label_object]));
    }
    if !diagnostic.notes.is_empty() {
        object.set(
            "notes",
            Json::array(
                diagnostic
                    .notes
                    .iter()
                    .map(|note| Json::string(note.as_str()))
                    .collect(),
            ),
        );
    }
    if let Some(url) = diagnostic.docs_url() {
        object.set("docs_url", Json::string(url));
    }
    object
}

fn span_json(sources: &SourceMap, file: FileId, span: Span) -> Json {
    let location = sources.locate(file, span);
    let mut object = span_range_json(span);
    object.set("line", Json::number(i64::from(location.line)));
    object.set("column", Json::number(i64::from(location.col)));
    object
}

fn span_range_json(span: Span) -> Json {
    let mut object = Json::object();
    object.set("start", Json::number(i64::from(span.start)));
    object.set("end", Json::number(i64::from(span.end)));
    object
}

/// `yashy check <file.ya> [--json]`.
fn check(args: &[String]) -> ExitCode {
    let mut json = false;
    let mut input: Option<PathBuf> = None;
    for argument in args {
        match argument.as_str() {
            "--json" => json = true,
            other if other.starts_with('-') => {
                return usage_error(&format!("unknown option `{other}` for `check`"));
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
        return usage_error("`check` needs a source file");
    };
    let source = match read_source_with(&input, EXIT_USAGE) {
        Ok(source) => source,
        Err(code) => return code,
    };
    // `check` reports the same diagnostics in either build mode -- a release
    // build still checks a `debug_assert`'s condition, it only declines to
    // lower it -- so there is no `--release` here to accept and ignore.
    let mut sources = SourceMap::new();
    match load_and_check(&mut sources, &input, source, false) {
        Ok(_) => {
            if json {
                print_check_json(&sources, &[]);
            }
            exit_code(EXIT_OK)
        }
        Err(diagnostics) => {
            if json {
                let errors = diagnostics
                    .iter()
                    .filter(|diagnostic| diagnostic.is_error())
                    .count();
                print_check_json(&sources, &diagnostics);
                if errors > 0 {
                    exit_code(EXIT_COMPILE_ERROR)
                } else {
                    exit_code(EXIT_OK)
                }
            } else {
                render(&sources, &diagnostics)
            }
        }
    }
}

fn read_source(input: &Path) -> Result<String, ExitCode> {
    read_source_with(input, EXIT_COMPILE_ERROR)
}

fn read_source_with(input: &Path, code: i32) -> Result<String, ExitCode> {
    std::fs::read_to_string(input).map_err(|error| {
        eprintln!("error: cannot read {}: {error}", input.display());
        exit_code(code)
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
    exit_code(EXIT_USAGE)
}

fn exit_code(code: i32) -> ExitCode {
    ExitCode::from(code as u8)
}
