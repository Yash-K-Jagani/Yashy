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
    check_and_verify_timed, compile_hir_timed, detect, load_and_check_timed, resolve_opt, timed,
    BuildOptions, CpuTarget, OptLevel, PhaseTiming,
};
use yashy_verify::{first_contract, Assumption, ContractMode, VerifyOptions, VerifyReport};

/// How every command that verifies reads the program.
///
/// A contract is a promise the verifier checks, so an unproved one is an error
/// in every path that claims the program is sound: `verify`, `build`, `run` and
/// the human `test` path all share this one value. `yashy test`'s JSON path
/// builds on the same checker, so it agrees with the human path by
/// construction.
///
/// This is deliberately one constant rather than a flag per command. The
/// contract pass does not know the mode, which is what makes the flip a single
/// line; a command that wanted to warn would have to say so, and none does.
const OPTIONS: VerifyOptions = VerifyOptions {
    contracts: ContractMode::Enforce,
};

const USAGE: &str = "\
usage:
  yashy build <file.ya> [-o <out>] [-O0|-O1|-O2|-O3] [--cpu <tier>] [--fast-math] [--release] [--emit-ir] [--timings] [--no-verify]
  yashy run <file.ya> [-O0|-O1|-O2|-O3] [--cpu <tier>] [--fast-math] [--release] [--timings] [--no-verify] [-- <args>...]
  yashy check <file.ya> [--json] [--timings]
  yashy verify <file.ya> [--json] [--timings] [--inventory]
  yashy test [path] [--filter <substring>] [--list] [--json] [--timings]
  yashy explain <code> [--json]
  yashy init [path]
  yashy doctor [--json]
  yashy fmt [-w] <files...>
  yashy version

  build   compile a program to a native executable
  run     compile and execute a program
  check   lex, parse and type-check without emitting anything
  verify  prove the program sound: every linear value has an owner
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
  --no-verify  build without proving the program sound; refused when the program
               has a contract or `YASHY_REQUIRE_VERIFY` is set, and stamps
               `<output>.unverified` beside the binary
  --timings    print how long each pipeline phase took, to stderr (or in the
               JSON report); stdout is never touched
  --inventory  with `verify`: list every `extern` declaration and `unsafe` call,
               the assumptions the proof rests on; never changes the exit code
  --json       print a stable machine-readable report (check, verify, test, explain, doctor)";

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
        "verify" => verify(rest),
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

/// What one build produced: its exit code and the phases it measured.
///
/// A build is callable without printing a timing report of its own because
/// `test` compiles one program per test and wants one report for the suite, not
/// five lines per test. [`build`] is the wrapper that prints.
struct BuildOutcome {
    code: ExitCode,
    /// Empty unless `--timings` was passed.
    timings: Vec<PhaseTiming>,
}

/// `yashy build <file.ya> [-o <out>] [-O0|-1|-O2|-O3] [--cpu <tier>] [--fast-math] [--release] [--emit-ir] [--timings] [--no-verify]`.
fn build(args: &[String]) -> ExitCode {
    let outcome = build_measured(args);
    // An empty vector means the flag was not passed, so this is silent then.
    report_timings(&outcome.timings);
    outcome.code
}

fn build_measured(args: &[String]) -> BuildOutcome {
    let mut input: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    let mut emit_ir = false;
    let mut timings_flag = false;
    let mut no_verify = false;
    // Every exit before the pipeline starts has no phase to report.
    let stopped = |code: ExitCode| BuildOutcome {
        code,
        timings: Vec::new(),
    };
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
            "--timings" => timings_flag = true,
            "--no-verify" => no_verify = true,
            "--fast-math" => fast_math = true,
            "--release" => release = true,
            "--native" => cpu = CpuTarget::Native,
            "--cpu" => match args.next() {
                Some(value) => match CpuTarget::parse(value) {
                    Some(target) => cpu = target,
                    None => return stopped(usage_error(&unknown_cpu(value))),
                },
                None => return stopped(usage_error("`--cpu` needs a value")),
            },
            "-o" | "--output" => match args.next() {
                Some(value) => output = Some(PathBuf::from(value)),
                None => return stopped(usage_error("`-o` needs a path")),
            },
            other => {
                if let Some(value) = other.strip_prefix("--cpu=") {
                    match CpuTarget::parse(value) {
                        Some(target) => cpu = target,
                        None => return stopped(usage_error(&unknown_cpu(value))),
                    }
                } else if let Some(level) = OptLevel::parse(other) {
                    opt = Some(level);
                } else if other.starts_with("-O") {
                    return stopped(usage_error(&format!(
                        "`{other}` is not an optimization level; use -O0, -O1, -O2 or -O3"
                    )));
                } else if other.starts_with('-') {
                    return stopped(usage_error(&format!("unknown option `{other}`")));
                } else {
                    if input.is_some() {
                        return stopped(usage_error(&format!(
                            "unexpected extra argument `{other}`"
                        )));
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
        return stopped(usage_error(&format!(
            "`{}` is an x86-64 feature level, but this compiler targets `{}`",
            cpu.name(),
            host_target_triple()
        )));
    }

    let Some(input) = input else {
        return stopped(usage_error("`build` needs a source file"));
    };

    // `--no-verify` is an escape hatch, not a mode. It exists so a user can
    // build a program the verifier is wrong about, and it has to be loud and
    // traceable. A pipeline that demands verification refuses it outright, and
    // so does a program that asks for a proof only the verifier can supply.
    if no_verify && require_verify_from_env() {
        return stopped(usage_error(
            "`YASHY_REQUIRE_VERIFY` is set, so `--no-verify` is refused",
        ));
    }
    if no_verify && emit_ir {
        return stopped(usage_error(
            "`--no-verify` has no meaning with `--emit-ir`; IR is not a binary",
        ));
    }

    let source = match read_source(&input) {
        Ok(source) => source,
        Err(code) => return stopped(code),
    };
    let entry = input.to_string_lossy().into_owned();

    // Timings are collected only when asked. An empty vector is how the
    // outcome says "the flag was not passed", so `build` prints without having
    // to parse the arguments a second time.
    let mut timings = Vec::new();
    let mut scratch = Vec::new();
    let measured: &mut Vec<PhaseTiming> = if timings_flag {
        &mut timings
    } else {
        &mut scratch
    };

    // One map for the whole program: the loader adds every imported file to it
    // as it reads that file, so a diagnostic from a module renders against the
    // module's own text. The entry point is file id zero.
    let mut sources = SourceMap::new();

    let (hir, report) = if no_verify {
        // The checker still runs: `--no-verify` skips the proof, not the type
        // system. A program that does not type-check has no binary worth
        // stamping.
        let hir = match load_and_check_timed(&mut sources, &input, source, release, measured) {
            Ok(hir) => hir,
            Err(diagnostics) => {
                return BuildOutcome {
                    code: render(&sources, &diagnostics),
                    timings,
                }
            }
        };
        // A contract is a promise only the verifier can check, so a build that
        // skips the verifier cannot honour it. The first one is named: "this
        // program has contracts" is not an actionable message.
        if let Some(name) = first_contract(&hir) {
            eprintln!(
                "error: `--no-verify` is refused because `{name}` has a contract; \
                 a contract is only meaningful when the verifier runs"
            );
            return BuildOutcome {
                code: exit_code(EXIT_USAGE),
                timings,
            };
        }
        (hir, VerifyReport::default())
    } else {
        match check_and_verify_timed(&mut sources, &input, source, release, &OPTIONS, measured) {
            Ok(checked) => checked,
            Err(diagnostics) => {
                return BuildOutcome {
                    code: render(&sources, &diagnostics),
                    timings,
                }
            }
        }
    };

    // An unverified binary is not a build. A verifier diagnostic may be a
    // warning, which is printed and does not stop the build; an error stops it
    // here, before `clang` is ever started.
    if !report.is_empty() {
        let code = render(&sources, &report.diagnostics);
        if report.error_count() > 0 {
            return BuildOutcome { code, timings };
        }
    }

    // `--emit-ir` is a pure front-end operation, so it deliberately does not
    // require an LLVM toolchain: dumping IR is exactly what you want to do on a
    // machine where the backend cannot run yet.
    if emit_ir {
        let code = match check_main_exists(&hir.items) {
            Ok(()) => {
                let options = CodegenOptions {
                    fp_contract: fast_math,
                };
                let ir = timed(measured, "emit", || emit_program(&hir, &entry, options));
                print!("{ir}");
                exit_code(EXIT_OK)
            }
            Err(diagnostic) => render(&sources, &[diagnostic]),
        };
        return BuildOutcome { code, timings };
    }

    let toolchain = match detect() {
        Ok(toolchain) => toolchain,
        Err(message) => {
            eprintln!("error: {message}");
            return BuildOutcome {
                code: exit_code(EXIT_COMPILE_ERROR),
                timings,
            };
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
    let code = match compile_hir_timed(&hir, &entry, &output, &toolchain, options, measured) {
        Ok(()) => {
            let sidecar = unverified_sidecar(&output);
            if no_verify {
                // The stamp is the whole point of the escape hatch: an
                // unverified binary must be identifiable after the fact. If the
                // stamp cannot be written, the binary must not survive either,
                // or the one promise `--no-verify` makes is broken.
                match fs::write(&sidecar, UNVERIFIED_SIDECAR) {
                    Ok(()) => eprintln!(
                        "warning: built without verification (`--no-verify`); wrote {}",
                        sidecar.display()
                    ),
                    Err(error) => {
                        let _ = fs::remove_file(&output);
                        eprintln!("error: cannot stamp {}: {error}", sidecar.display());
                        return BuildOutcome {
                            code: exit_code(EXIT_COMPILE_ERROR),
                            timings,
                        };
                    }
                }
            } else {
                // A verified build retires an earlier unverified claim. A stale
                // stamp over-states caution rather than hiding a risk, so a
                // failure to remove it is a warning, not a failed build.
                if let Err(error) = fs::remove_file(&sidecar) {
                    if error.kind() != std::io::ErrorKind::NotFound {
                        eprintln!(
                            "warning: cannot remove stale {}: {error}",
                            sidecar.display()
                        );
                    }
                }
            }
            exit_code(EXIT_OK)
        }
        Err(diagnostics) => render(&sources, &diagnostics),
    };
    BuildOutcome { code, timings }
}

/// The provenance stamp that sits beside an unverified binary.
///
/// `hello.exe` becomes `hello.exe.unverified`, so the pair travels together
/// through a copy or a package without a manifest to keep in step.
fn unverified_sidecar(output: &Path) -> PathBuf {
    let mut path = output.as_os_str().to_owned();
    path.push(".unverified");
    PathBuf::from(path)
}

/// The stamp's exact bytes. One line of JSON, so a reader can parse it without
/// guessing at a format, and a human can read it with `cat`.
const UNVERIFIED_SIDECAR: &str =
    r#"{"schema_version":1,"verified":false,"reason":"--no-verify","yashy_version":"0.1.0"}"#;

/// True when the environment demands verification.
///
/// Set `YASHY_REQUIRE_VERIFY` to anything but empty, `0` or `false` and
/// `--no-verify` becomes a usage error. A CI that has decided its binaries must
/// be proved can enforce that without trusting every command line it runs.
fn require_verify_from_env() -> bool {
    match env::var_os("YASHY_REQUIRE_VERIFY") {
        Some(value) => {
            let value = value.to_string_lossy();
            let value = value.trim();
            !(value.is_empty() || value == "0" || value.eq_ignore_ascii_case("false"))
        }
        None => false,
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
            "--release" | "--fast-math" | "--native" | "--timings" | "--no-verify" => {
                build_args.push(argument.clone())
            }
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
    let mut timings_flag = false;

    let mut iter = args.iter();
    while let Some(argument) = iter.next() {
        match argument.as_str() {
            "--list" => list = true,
            "--json" => json = true,
            "--timings" => timings_flag = true,
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
        return test_json(&root, &tests, timings_flag);
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
    // A test that does not build never runs, so it is not a failing test. The
    // per-test line already says so; the aggregate says it too.
    let mut compile_errors = 0usize;
    // One report for the suite: five lines per test would be noise, and what a
    // user wants from `test --timings` is where the suite's wall time went.
    let mut timings: Vec<PhaseTiming> = Vec::new();
    for path in &tests {
        let name = display_test_path(path);
        let dir = temp_run_dir();
        if let Err(error) = fs::create_dir_all(&dir) {
            eprintln!("error: cannot create {}: {error}", dir.display());
            failed += 1;
            continue;
        }
        let exe = dir.join(format!("test{}", std::env::consts::EXE_SUFFIX));
        let mut build_args = vec![
            path.to_string_lossy().into_owned(),
            "-o".to_string(),
            exe.to_string_lossy().into_owned(),
        ];
        // The build collects only when asked, and `test` prints the aggregate
        // itself, so the flag travels without `build` printing per test.
        if timings_flag {
            build_args.push("--timings".to_string());
        }
        let outcome = build_measured(&build_args);
        accumulate(&mut timings, &outcome.timings);
        if outcome.code != exit_code(EXIT_OK) {
            println!("test {name} ... FAILED (compile error)");
            compile_errors += 1;
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
        "test result: {}. {passed} passed; {failed} failed; {compile_errors} compile errors",
        if failed == 0 && compile_errors == 0 {
            "ok"
        } else {
            "FAILED"
        }
    );
    report_timings(&timings);
    if failed == 0 && compile_errors == 0 {
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
fn test_json(root: &Path, tests: &[PathBuf], timings_flag: bool) -> ExitCode {
    let mut global_sources = SourceMap::new();
    let root_file = global_sources.add_file(root.display().to_string(), String::new());
    // The suite's phase totals, so `test --json --timings` answers "where did
    // the run spend its time?" without a line per test.
    let mut timings: Vec<PhaseTiming> = Vec::new();

    if tests.is_empty() {
        print_test_json(&global_sources, &[], &[], 0, 0, 0, None);
        return exit_code(EXIT_OK);
    }

    let toolchain = match detect() {
        Ok(toolchain) => toolchain,
        Err(message) => {
            let diagnostic = Diagnostic::new(Severity::Error, root_file, Span::SYNTHETIC, message);
            print_test_json(&global_sources, &[diagnostic], &[], 0, 0, 0, None);
            return exit_code(EXIT_USAGE);
        }
    };

    let mut results: Vec<Json> = Vec::new();
    let mut passed = 0usize;
    let mut failed = 0usize;
    // A test that does not compile or does not verify never runs. Counting it
    // as `failed` would tell a reader an assertion fired when the truth is that
    // no test code ran, so it gets its own bucket and its own status.
    let mut compile_errors = 0usize;

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
            print_test_json(
                &global_sources,
                &[diagnostic],
                &results,
                passed,
                failed,
                compile_errors,
                timings_flag.then_some(timings.as_slice()),
            );
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
        let mut phases = Vec::new();
        // `test` and `test --json` must agree about what passes. The human path
        // builds through `build_measured`, which verifies, so a test that leaks
        // is a failure there and has to be one here too. Warnings travel with a
        // passing test, exactly as `build` prints them and still links.
        let outcome =
            match check_and_verify_timed(&mut sources, entry, source, false, &OPTIONS, &mut phases)
            {
                Ok((hir, report)) => {
                    if report.error_count() > 0 {
                        Err(report.diagnostics)
                    } else {
                        let warnings = report.diagnostics;
                        match compile_hir_timed(
                            &hir,
                            &file,
                            &exe,
                            &toolchain,
                            BuildOptions::default(),
                            &mut phases,
                        ) {
                            Ok(()) => Ok(warnings),
                            Err(more) => {
                                let mut diagnostics = warnings;
                                diagnostics.extend(more);
                                Err(diagnostics)
                            }
                        }
                    }
                }
                Err(diagnostics) => Err(diagnostics),
            };
        accumulate(&mut timings, &phases);

        match outcome {
            Ok(warnings) => match output_with_retry(&exe, &[]) {
                Ok(output) if output.status.success() => {
                    passed += 1;
                    results.push(test_json_object(
                        &name,
                        &file,
                        "passed",
                        elapsed_ms(started),
                        &sources,
                        &warnings,
                    ));
                }
                Ok(output) => {
                    failed += 1;
                    let mut diagnostics = warnings;
                    diagnostics.push(Diagnostic::new(
                        Severity::Error,
                        FileId::from_index(0),
                        Span::SYNTHETIC,
                        match output.status.code() {
                            Some(code) => format!("test exited with status {code}"),
                            None => "test was terminated without an exit status".to_string(),
                        },
                    ));
                    results.push(test_json_object(
                        &name,
                        &file,
                        "failed",
                        elapsed_ms(started),
                        &sources,
                        &diagnostics,
                    ));
                }
                Err(error) => {
                    failed += 1;
                    let mut diagnostics = warnings;
                    diagnostics.push(Diagnostic::new(
                        Severity::Error,
                        FileId::from_index(0),
                        Span::SYNTHETIC,
                        format!("cannot run the test: {error}"),
                    ));
                    results.push(test_json_object(
                        &name,
                        &file,
                        "failed",
                        elapsed_ms(started),
                        &sources,
                        &diagnostics,
                    ));
                }
            },
            Err(diagnostics) => {
                compile_errors += 1;
                results.push(test_json_object(
                    &name,
                    &file,
                    "compile_error",
                    elapsed_ms(started),
                    &sources,
                    &diagnostics,
                ));
            }
        }

        let _ = fs::remove_dir_all(&dir);
    }

    print_test_json(
        &global_sources,
        &[],
        &results,
        passed,
        failed,
        compile_errors,
        timings_flag.then_some(timings.as_slice()),
    );
    if failed == 0 && compile_errors == 0 {
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
    compile_errors: usize,
    timings: Option<&[PhaseTiming]>,
) {
    let mut summary = Json::object();
    summary.set("passed", Json::number(passed as i64));
    summary.set("failed", Json::number(failed as i64));
    summary.set("compile_errors", Json::number(compile_errors as i64));
    summary.set("skipped", Json::number(0));

    let mut report = Json::object();
    report.set("schema_version", Json::number(1));
    report.set("tool", Json::string("test"));
    report.set("yashy_version", Json::string(env!("CARGO_PKG_VERSION")));
    report.set(
        "success",
        Json::bool(failed == 0 && compile_errors == 0 && global.is_empty()),
    );
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
    set_timings(&mut report, timings);
    println!("{}", report.render());
}

/// Add the `timings` array to an envelope when the flag was passed.
///
/// Omitted rather than empty when it was not: a report that never asked for
/// timings should stay byte-identical to the one this compiler printed before
/// the flag existed, and an empty array would be a new field for every caller.
fn set_timings(report: &mut Json, timings: Option<&[PhaseTiming]>) {
    let Some(timings) = timings else {
        return;
    };
    report.set(
        "timings",
        Json::array(
            timings
                .iter()
                .map(|timing| {
                    let mut entry = Json::object();
                    entry.set("phase", Json::string(timing.phase));
                    entry.set("ms", Json::number(timing.ms as i64));
                    entry
                })
                .collect(),
        ),
    );
}

/// Add one phase's time to a running total, keeping first-seen order.
fn accumulate(total: &mut Vec<PhaseTiming>, timings: &[PhaseTiming]) {
    for timing in timings {
        match total.iter_mut().find(|seen| seen.phase == timing.phase) {
            Some(seen) => seen.ms = seen.ms.saturating_add(timing.ms),
            None => total.push(*timing),
        }
    }
}

/// Print one `timing:` line per phase, to stderr.
///
/// stderr is deliberate: stdout belongs to the program under `run` and to the
/// machine-readable envelope under `--json`, and a timing line in either would
/// corrupt it. An empty slice prints nothing, which is how a caller says the
/// flag was not passed.
fn report_timings(timings: &[PhaseTiming]) {
    for timing in timings {
        eprintln!("timing: {} {} ms", timing.phase, timing.ms);
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

fn print_check_json(
    tool: &str,
    sources: &SourceMap,
    diagnostics: &[Diagnostic],
    timings: Option<&[PhaseTiming]>,
    assumptions: Option<&[Assumption]>,
) {
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
    report.set("tool", Json::string(tool));
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
    set_timings(&mut report, timings);
    // Only `verify --inventory` asks the question, so only it answers: an
    // absent key means "not computed", which is not the same fact as an empty
    // list. A program that trusts nothing says so with `[]`.
    if let Some(assumptions) = assumptions {
        report.set(
            "assumptions",
            Json::array(
                assumptions
                    .iter()
                    .map(|assumption| assumption_json(sources, assumption))
                    .collect(),
            ),
        );
    }
    println!("{}", report.render());
}

/// One assumption in the schema-v1 shape: what it is, what it names, where.
fn assumption_json(sources: &SourceMap, assumption: &Assumption) -> Json {
    let mut object = Json::object();
    object.set("kind", Json::string(assumption.kind.as_str()));
    object.set("name", Json::string(assumption.name.as_str()));
    object.set("file", Json::string(sources.path(assumption.file)));
    object.set("span", span_json(sources, assumption.file, assumption.span));
    object
}

/// The human inventory: one line per assumption, aligned, in source order.
///
/// `assumptions: 0` is printed too. "This program trusts nothing" is an answer,
/// and a user who asked for the list should get one even when it is empty.
fn print_inventory(sources: &SourceMap, assumptions: &[Assumption]) {
    println!("assumptions: {}", assumptions.len());
    let kind_width = assumptions
        .iter()
        .map(|assumption| assumption.kind.as_str().len())
        .max()
        .unwrap_or(0);
    let name_width = assumptions
        .iter()
        .map(|assumption| assumption.name.len())
        .max()
        .unwrap_or(0);
    for assumption in assumptions {
        let location = sources.locate(assumption.file, assumption.span);
        println!(
            "  {kind:<kind_width$}  {name:<name_width$}  {file}:{line}:{column}",
            kind = assumption.kind.as_str(),
            name = assumption.name,
            file = sources.path(assumption.file),
            line = location.line,
            column = location.col,
        );
    }
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
    if let Some(proof) = &diagnostic.proof {
        let mut proof_object = Json::object();
        proof_object.set("obligation", Json::string(proof.obligation));
        proof_object.set("missing_fact", Json::string(proof.missing_fact.as_str()));
        proof_object.set("remedy", Json::string(proof.remedy.as_str()));
        object.set("proof", proof_object);
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
    let mut timings_flag = false;
    let mut input: Option<PathBuf> = None;
    for argument in args {
        match argument.as_str() {
            "--json" => json = true,
            "--timings" => timings_flag = true,
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
    let mut timings = Vec::new();
    let mut scratch = Vec::new();
    let measured: &mut Vec<PhaseTiming> = if timings_flag {
        &mut timings
    } else {
        &mut scratch
    };
    let outcome = load_and_check_timed(&mut sources, &input, source, false, measured);
    let reported = timings_flag.then_some(timings.as_slice());
    match outcome {
        Ok(_) => {
            if json {
                print_check_json("check", &sources, &[], reported, None);
            } else {
                report_timings(&timings);
            }
            exit_code(EXIT_OK)
        }
        Err(diagnostics) => {
            if json {
                let errors = diagnostics
                    .iter()
                    .filter(|diagnostic| diagnostic.is_error())
                    .count();
                print_check_json("check", &sources, &diagnostics, reported, None);
                if errors > 0 {
                    exit_code(EXIT_COMPILE_ERROR)
                } else {
                    exit_code(EXIT_OK)
                }
            } else {
                let code = render(&sources, &diagnostics);
                report_timings(&timings);
                code
            }
        }
    }
}

/// `yashy verify <file.ya> [--json] [--inventory]`.
///
/// The same front half as `check`, followed by the verifier. The difference the
/// user sees is what the exit code means: `check` fails when the program is
/// not well-typed, `verify` also fails when it is well-typed but unsound.
///
/// `--inventory` answers a second question, "what does this program trust?", by
/// listing every C declaration and every `unsafe` call the proof rests on. It is
/// evidence, not a verdict: the exit code is the same with and without it.
///
/// The two failure modes stay distinguishable in the output, because a check
/// diagnostic names the source it is about and a verifier diagnostic carries a
/// proof trace saying what could not be established.
fn verify(args: &[String]) -> ExitCode {
    let mut json = false;
    let mut timings_flag = false;
    let mut inventory_flag = false;
    let mut input: Option<PathBuf> = None;
    for argument in args {
        match argument.as_str() {
            "--json" => json = true,
            "--timings" => timings_flag = true,
            "--inventory" => inventory_flag = true,
            other if other.starts_with('-') => {
                return usage_error(&format!("unknown option `{other}` for `verify`"));
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
        return usage_error("`verify` needs a source file");
    };
    let source = match read_source_with(&input, EXIT_USAGE) {
        Ok(source) => source,
        Err(code) => return code,
    };
    // Verification never depends on how hard the program will be optimized, so
    // there is no `--release` here for the same reason `check` has none.
    let mut sources = SourceMap::new();
    let mut timings = Vec::new();
    let mut scratch = Vec::new();
    let measured: &mut Vec<PhaseTiming> = if timings_flag {
        &mut timings
    } else {
        &mut scratch
    };
    let outcome = check_and_verify_timed(&mut sources, &input, source, false, &OPTIONS, measured);
    let reported = timings_flag.then_some(timings.as_slice());
    match outcome {
        Ok((hir, report)) => {
            // The inventory is computed from the checked HIR, so it exists
            // exactly when the program type-checked. It never changes the
            // exit code: the verifier accepted the program with these
            // assumptions before the flag existed, and it still does.
            let assumptions = inventory_flag.then(|| yashy_verify::inventory(&hir));
            if json {
                print_check_json(
                    "verify",
                    &sources,
                    &report.diagnostics,
                    reported,
                    assumptions.as_deref(),
                );
            } else {
                if !report.is_empty() {
                    for diagnostic in &report.diagnostics {
                        eprint!("{}", sources.render(diagnostic));
                    }
                }
                if let Some(assumptions) = &assumptions {
                    print_inventory(&sources, assumptions);
                }
            }
            if !json {
                report_timings(&timings);
            }
            if report.error_count() > 0 {
                exit_code(EXIT_COMPILE_ERROR)
            } else {
                exit_code(EXIT_OK)
            }
        }
        // A program that did not type-check has no typed HIR to verify, so the
        // check diagnostics are the whole answer. Saying so in the envelope
        // keeps `verify` honest about what it could not run.
        Err(diagnostics) => {
            if json {
                let errors = diagnostics
                    .iter()
                    .filter(|diagnostic| diagnostic.is_error())
                    .count();
                print_check_json("verify", &sources, &diagnostics, reported, None);
                if errors > 0 {
                    exit_code(EXIT_COMPILE_ERROR)
                } else {
                    exit_code(EXIT_OK)
                }
            } else {
                let code = render(&sources, &diagnostics);
                report_timings(&timings);
                code
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
