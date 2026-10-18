//! The `otter` command line interface.
//!
//! ```text
//! otter build <file.ot> [-o <out>] [--emit-ir]
//! otter check <file.ot>
//! otter version
//! ```
//!
//! Argument parsing is hand-rolled on purpose: three subcommands and two flags
//! do not need a parser generator, and every dependency in this workspace is
//! one more thing between a user and a compiler that builds offline.

use std::env;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use otter_codegen::{check_main_exists, emit_program};
use otter_common::{Diagnostic, EXIT_COMPILE_ERROR, EXIT_OK, SourceMap};
use otter_driver::{compile_to_binary, detect, OptLevel};
use otter_lexer::Lexer;
use otter_parser::Parser;
use otter_sema::{check_program, HirProgram};

const USAGE: &str = "\
usage:
  otter build <file.ot> [-o <out>] [-O0|-O1|-O2|-O3] [--emit-ir]
  otter check <file.ot>
  otter version

  build   compile a program to a native executable
  check   lex, parse and type-check without emitting anything
  -O<n>       LLVM optimization level for the generated code (default -O2)
  --emit-ir   print the generated LLVM IR to stdout and stop";

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
        other => {
            eprintln!("error: unknown command `{other}`\n{USAGE}");
            exit_code(EXIT_COMPILE_ERROR)
        }
    }
}

/// `otter build <file.ot> [-o <out>] [-O0|-O1|-O2|-O3] [--emit-ir]`.
fn build(args: &[String]) -> ExitCode {
    let mut input: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    let mut emit_ir = false;
    // Defaulting to `-O2` is the whole point of exposing this: the compiler
    // emits unoptimized IR so it stays readable, and hands the optimization to
    // LLVM. A build that passes no level gets `-O0` and a binary an order of
    // magnitude slower than the same source deserves.
    let mut opt = OptLevel::default();

    let mut args = args.iter();
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--emit-ir" => emit_ir = true,
            "-o" | "--output" => match args.next() {
                Some(value) => output = Some(PathBuf::from(value)),
                None => return usage_error("`-o` needs a path"),
            },
            other => {
                if let Some(level) = OptLevel::parse(other) {
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

    let Some(input) = input else {
        return usage_error("`build` needs a source file");
    };
    let source = match read_source(&input) {
        Ok(source) => source,
        Err(code) => return code,
    };
    let entry = input.to_string_lossy().into_owned();

    // `--emit-ir` is a pure front-end operation, so it deliberately does not
    // require an LLVM toolchain: dumping IR is exactly what you want to do on a
    // machine where the backend cannot run yet.
    if emit_ir {
        return match front_end(&source, &entry) {
            Ok(hir) => match check_main_exists(&hir.items) {
                Ok(()) => {
                    print!("{}", emit_program(&hir, &entry));
                    exit_code(EXIT_OK)
                }
                Err(diagnostic) => render(&source, &entry, &[diagnostic]),
            },
            Err(diagnostics) => render(&source, &entry, &diagnostics),
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
    match compile_to_binary(&source, &entry, &output, &toolchain, opt) {
        Ok(()) => exit_code(EXIT_OK),
        Err(diagnostics) => render(&source, &entry, &diagnostics),
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
    let entry = input.to_string_lossy().into_owned();
    match front_end(&source, &entry) {
        Ok(_) => exit_code(EXIT_OK),
        Err(diagnostics) => render(&source, &entry, &diagnostics),
    }
}

/// Lex, parse and type-check `source`.
///
/// The first failing stage returns its diagnostics; a stage is never run on
/// input the previous one rejected.
fn front_end(source: &str, entry: &str) -> Result<HirProgram, Vec<Diagnostic>> {
    let mut sources = SourceMap::new();
    let file = sources.add_file(entry, source.to_string());
    let tokens = Lexer::new(source).tokenize()?;
    let mut parser = Parser::new(tokens, &sources, file);
    let program = parser.parse_program()?;
    check_program(&program, &sources, file)
}

fn read_source(input: &Path) -> Result<String, ExitCode> {
    std::fs::read_to_string(input).map_err(|error| {
        eprintln!("error: cannot read {}: {error}", input.display());
        exit_code(EXIT_COMPILE_ERROR)
    })
}

/// Render each diagnostic with its source line and caret, to stderr.
fn render(source: &str, entry: &str, diagnostics: &[Diagnostic]) -> ExitCode {
    // The diagnostics name file id zero of a fresh map, which is `entry`'s
    // text; adding it here in the same position is what makes the carets line
    // up with what the user wrote.
    let mut sources = SourceMap::new();
    let _ = sources.add_file(entry, source.to_string());
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

fn usage_error(message: &str) -> ExitCode {
    eprintln!("error: {message}\n{USAGE}");
    exit_code(EXIT_COMPILE_ERROR)
}

fn exit_code(code: i32) -> ExitCode {
    ExitCode::from(code as u8)
}
