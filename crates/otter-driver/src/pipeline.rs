//! The compilation pipeline: Otter source in, native executable out.
//!
//! The stages run in one order and stop at the first failure:
//!
//! ```text
//! lex -> parse -> check -> emit IR -> clang -c (object) -> clang (link)
//! ```
//!
//! Nothing downstream of a failing stage runs, so a program with a syntax
//! error never reaches codegen and a program with a type error is never handed
//! to `clang`. This is what keeps a confusing linker message from ever being
//! the user's first feedback about a typo.
//!
//! # Where the runtime comes from
//!
//! The last `clang` invocation compiles the *runtime C source* alongside the
//! generated object, rather than linking a prebuilt archive. [`RUNTIME_C_SOURCE`]
//! is embedded in the compiler binary, so there is no library search path to
//! configure, no archive to go stale, and the runtime is always the one this
//! compiler was built and tested with.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use otter_codegen::{check_main_exists, emit_program, host_target_triple};
use otter_common::{Diagnostic, FileId, Severity, SourceMap, Span};
use otter_lexer::Lexer;
use otter_parser::Parser;
use otter_runtime::{RUNTIME_C_SOURCE, RUNTIME_HEADER, RUNTIME_HEADER_NAME, RUNTIME_SOURCE_NAME};
use otter_sema::check_program;

use crate::toolchain::Toolchain;

/// Compile `source` to a native executable at `out`.
///
/// `entry_name` is the path reported in diagnostics and in the IR's
/// `source_filename`; it is what the user typed, not a canonicalised path.
///
/// Every diagnostic refers to file id **zero** of a fresh [`SourceMap`], which
/// is `entry_name`'s text. A caller that wants to render them builds its own
/// map by adding the same source first — the driver's CLI does exactly that.
///
/// Returns `Err` with every diagnostic of the first failing stage; on success,
/// `Ok(())` and no output at all (the caller decides what to print).
pub fn compile_to_binary(
    source: &str,
    entry_name: &str,
    out: &Path,
    toolchain: &Toolchain,
) -> Result<(), Vec<Diagnostic>> {
    let mut sources = SourceMap::new();
    let file = sources.add_file(entry_name, source.to_string());

    let tokens = Lexer::new(source).tokenize()?;
    let mut parser = Parser::new(tokens, &sources, file);
    let program = parser.parse_program()?;
    let hir = check_program(&program, &sources, file)?;

    if let Err(diagnostic) = check_main_exists(&hir.items) {
        return Err(vec![diagnostic]);
    }

    let ir_text = emit_program(&hir, entry_name);

    // The IR next to the output is a debugging aid, not a temporary: when a
    // program misbehaves, "what did we actually hand to LLVM?" is the first
    // question, and it should not require re-running with a flag.
    let ir_path = with_extension(out, "ll");
    write_or_report(&ir_path, ir_text.as_bytes(), entry_name)?;

    let work_dir = out.parent().unwrap_or_else(|| Path::new(".")).to_path_buf();
    let runtime_dir = work_dir.join(format!(
        "{}.otter",
        out.file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| "out".to_string())
    ));
    let runtime_source = runtime_dir.join(RUNTIME_SOURCE_NAME);
    ensure_dir(&runtime_dir, entry_name)?;
    write_or_report(&runtime_source, RUNTIME_C_SOURCE.as_bytes(), entry_name)?;
    write_or_report(
        &runtime_dir.join(RUNTIME_HEADER_NAME),
        RUNTIME_HEADER.as_bytes(),
        entry_name,
    )?;

    let object_path = with_extension(out, "o");
    let target = host_target_triple();

    // 1. IR text -> object file.
    run_tool(
        toolchain,
        &[
            "-c".to_string(),
            display(&ir_path),
            "-o".to_string(),
            display(&object_path),
            format!("--target={target}"),
        ],
        entry_name,
        "assembling the generated IR",
    )?;

    // 2. object + runtime -> executable.
    let mut link_args = vec![
        display(&object_path),
        display(&runtime_source),
        format!("-I{}", display(&runtime_dir)),
        "-o".to_string(),
        display(out),
        format!("--target={target}"),
    ];
    if !cfg!(windows) {
        // The runtime only needs stdio, but a Linux link of a freestanding
        // object may still pull in libm and the thread runtime.
        link_args.push("-lpthread".to_string());
        link_args.push("-ldl".to_string());
        link_args.push("-lm".to_string());
    }
    run_tool(
        toolchain,
        &link_args,
        entry_name,
        "linking the executable",
    )?;

    Ok(())
}

/// Run the toolchain executable with `args`, turning a non-zero exit or a
/// spawn failure into a diagnostic rather than raw child output.
///
/// `what` names the step for the message, so a link failure says which of the
/// two invocations failed.
fn run_tool(
    toolchain: &Toolchain,
    args: &[String],
    entry_name: &str,
    what: &str,
) -> Result<(), Vec<Diagnostic>> {
    let output = match Command::new(&toolchain.llc).args(args).output() {
        Ok(output) => output,
        Err(error) => {
            return Err(vec![toolchain_diagnostic(
                entry_name,
                format!(
                    "could not run `{}` while {what}: {error}",
                    toolchain.llc.display()
                ),
            )])
        }
    };

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let summary = first_meaningful_line(&stderr).unwrap_or("no output from the tool");
    Err(vec![toolchain_diagnostic(
        entry_name,
        format!("`{}` failed while {what}: {summary}", toolchain.llc.display()),
    )])
}

/// The runtime/IR diagnostic: about the compiler's own output, so it uses the
/// synthetic span and renders at the start of the file rather than underlining
/// a line of the user's source that has nothing to do with the failure.
fn toolchain_diagnostic(entry_name: &str, message: String) -> Diagnostic {
    Diagnostic {
        severity: Severity::Error,
        message,
        file: FileId::from_index(0),
        span: Span::SYNTHETIC,
        label: Some(format!(
            "while compiling `{entry_name}`; rerun with --emit-ir to see the generated IR"
        )),
    }
}

/// The first non-empty, non-`warning:` line of tool output.
///
/// `clang` prints its warnings and then its error, and the error is the line
/// worth showing. Falls back to the first non-empty line so a tool that prints
/// something unusual is still quoted rather than swallowed.
fn first_meaningful_line(stderr: &str) -> Option<&str> {
    let mut first = None;
    for line in stderr.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if first.is_none() {
            first = Some(trimmed);
        }
        if trimmed.starts_with("error") {
            return Some(trimmed);
        }
    }
    first
}

/// `out` with `extension` replaced, e.g. `hello.exe` -> `hello.ll`.
fn with_extension(out: &Path, extension: &str) -> PathBuf {
    out.with_extension(extension)
}

fn display(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn ensure_dir(dir: &Path, entry_name: &str) -> Result<(), Vec<Diagnostic>> {
    fs::create_dir_all(dir).map_err(|error| {
        vec![toolchain_diagnostic(
            entry_name,
            format!("could not create {}: {error}", dir.display()),
        )]
    })
}

fn write_or_report(path: &Path, bytes: &[u8], entry_name: &str) -> Result<(), Vec<Diagnostic>> {
    fs::write(path, bytes).map_err(|error| {
        vec![toolchain_diagnostic(
            entry_name,
            format!("could not write {}: {error}", path.display()),
        )]
    })
}
