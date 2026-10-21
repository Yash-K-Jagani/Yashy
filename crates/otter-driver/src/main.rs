//! The `otter` command line interface.
//!
//! ```text
//! otter build <file.ot> [-o <out>] [--emit-ir] [--cpu <tier>] [--fast-math] [--release]
//! otter check <file.ot>
//! otter fmt [-w] <files...>
//! otter version
//! ```
//!
//! Argument parsing is hand-rolled on purpose: four subcommands and a handful
//! of flags do not need a parser generator, and every dependency in this
//! workspace is one more thing between a user and a compiler that builds
//! offline.

use std::env;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use otter_codegen::{check_main_exists, emit_program, host_target_triple, CodegenOptions};
use otter_common::{Diagnostic, EXIT_COMPILE_ERROR, EXIT_OK, SourceMap};
use otter_driver::{compile_hir, detect, load_and_check, BuildOptions, CpuTarget, OptLevel};

const USAGE: &str = "\
usage:
  otter build <file.ot> [-o <out>] [-O0|-O1|-O2|-O3] [--cpu <tier>] [--fast-math] [--release] [--emit-ir]
  otter check <file.ot>
  otter fmt [-w] <files...>
  otter version

  build   compile a program to a native executable
  check   lex, parse and type-check without emitting anything
  fmt     rewrite source files in the canonical style; -w writes them back
  -O<n>        LLVM optimization level for the generated code (default -O2)
  --cpu <tier> instruction set the program may use, one of:
               portable (default), x86-64-v2, x86-64-v3, x86-64-v4, native
               (`--native` is a synonym for `--cpu native`)
  --fast-math  allow fused multiply-add; faster float code, different results
  --release    a release build: `debug_assert` is compiled out (the optimization
               level is unchanged; `-O3 --release` is what a shipped build wants)
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
            println!("otter {}", env!("CARGO_PKG_VERSION"));
            exit_code(EXIT_OK)
        }
        "help" | "--help" | "-h" => {
            println!("{USAGE}");
            exit_code(EXIT_OK)
        }
        "build" => build(rest),
        "check" => check(rest),
        "fmt" => fmt(rest),
        other => {
            eprintln!("error: unknown command `{other}`\n{USAGE}");
            exit_code(EXIT_COMPILE_ERROR)
        }
    }
}

/// `otter build <file.ot> [-o <out>] [-O0|-O1|-O2|-O3] [--cpu <tier>] [--fast-math] [--release] [--emit-ir]`.
fn build(args: &[String]) -> ExitCode {
    let mut input: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    let mut emit_ir = false;
    // Defaulting to `-O2` is the whole point of exposing this: the compiler
    // emits unoptimized IR so it stays readable, and hands the optimization to
    // LLVM. A build that passes no level gets `-O0` and a binary an order of
    // magnitude slower than the same source deserves.
    let mut opt = OptLevel::default();
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
    // A release build drops `debug_assert`. It is deliberately not tied to the
    // optimization level: `-O2` is already the default, so a flag that meant "be
    // faster" would be indistinguishable from saying nothing, and the two
    // questions -- how hard to optimize, and whether this is a build a user runs
    // -- are genuinely different ones.
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
                    opt = level;
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

/// `otter check <file.ot>`.
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

/// `otter fmt [-w] <files...>`.
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
        match otter_fmt::format(&source) {
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
