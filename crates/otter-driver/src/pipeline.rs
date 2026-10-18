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
//! The runtime is the *runtime C source*, [`RUNTIME_C_SOURCE`], embedded in the
//! compiler binary: there is no library search path to configure and the
//! runtime is always the one this compiler was built and tested with.
//!
//! At link time the driver needs that source as an object file. Compiling it
//! costs about 135 ms of a 400 ms build, and its bytes never change between
//! builds, so the object is cached — see `cached_runtime_object`. The cache
//! is keyed on the exact bytes it was built from, so it cannot go stale: a
//! changed runtime is a different key and a different object. When the cache
//! cannot be used for any reason, the driver falls back to compiling the source
//! as part of the link, which is what it did before there was a cache.

use std::env;
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

/// The `clang` optimization level applied to the generated IR.
///
/// The compiler emits unoptimized IR on purpose: the textual IR is a debugging
/// aid and staying close to the source is what makes it readable. All the
/// optimizing happens in LLVM, so "how fast is an Otter binary" is decided here
/// rather than in `otter-codegen`. The default is therefore `-O2`; a build that
/// never passes a level hands `clang` `-O0`, which is roughly an order of
/// magnitude of runtime performance left on the table for no benefit.
///
/// A closed set rather than a `u8`, so an out-of-range level is unrepresentable
/// once the CLI has parsed it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OptLevel {
    /// `-O0`. Fastest to compile; the level to use when reading the IR next to
    /// its machine code.
    None,
    /// `-O1`.
    Less,
    /// `-O2`. The default.
    #[default]
    Full,
    /// `-O3`.
    Aggressive,
}

impl OptLevel {
    /// The four levels `clang` accepts, `-O0` through `-O3`.
    pub const ALL: [OptLevel; 4] = [
        OptLevel::None,
        OptLevel::Less,
        OptLevel::Full,
        OptLevel::Aggressive,
    ];

    /// Parse a `-O<n>` argument. `None` for anything else, so the caller reports
    /// a usage error rather than guessing.
    pub fn parse(argument: &str) -> Option<Self> {
        match argument {
            "-O0" => Some(OptLevel::None),
            "-O1" => Some(OptLevel::Less),
            "-O2" => Some(OptLevel::Full),
            "-O3" => Some(OptLevel::Aggressive),
            _ => None,
        }
    }

    /// The spelling handed to `clang`.
    pub fn flag(self) -> &'static str {
        match self {
            OptLevel::None => "-O0",
            OptLevel::Less => "-O1",
            OptLevel::Full => "-O2",
            OptLevel::Aggressive => "-O3",
        }
    }
}

/// Which instruction set the generated program may use.
///
/// The baseline of a `--target` triple is deliberately conservative: an
/// `x86_64` binary may only use what every x86-64 CPU has, so it runs anywhere.
/// That costs real performance — a loop the backend would otherwise vectorise
/// four integers at a time is held to two — but the alternative, emitting
/// instructions for the *build* machine, produces a binary that dies with an
/// illegal instruction on an older CPU.
///
/// This is therefore opt-in, and [`CpuTarget::Portable`] is the default. Asking
/// for [`CpuTarget::Native`] is the user saying "this program is for this
/// machine", which is what makes it safe to un-conserve the target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CpuTarget {
    /// The triple's baseline: any CPU of that architecture can run the result.
    #[default]
    Portable,
    /// `-march=native`: the CPU this compiler is running on.
    Native,
}

impl CpuTarget {
    /// The extra `clang` arguments that select this target.
    ///
    /// Empty for the baseline, so a portable build passes nothing and behaves
    /// exactly as it did before this choice existed.
    pub fn flags(self) -> &'static [&'static str] {
        match self {
            CpuTarget::Portable => &[],
            CpuTarget::Native => &["-march=native"],
        }
    }
}

/// Where compiled runtime objects are reused between builds.
///
/// `OTTER_CACHE_DIR` overrides the location, which is what a build system or a
/// test that wants a private cache sets. Otherwise it follows the platform's
/// convention: `%LOCALAPPDATA%` on Windows, `XDG_CACHE_HOME` or `~/.cache`
/// elsewhere. The temp directory is the last resort, because a cache that
/// cannot be written is only a lost optimization and not a failure — see
/// [`cached_runtime_object`].
fn runtime_cache_dir() -> Option<PathBuf> {
    if let Some(dir) = env::var_os("OTTER_CACHE_DIR") {
        if !dir.is_empty() {
            return Some(PathBuf::from(dir));
        }
    }
    if let Some(local) = env::var_os("LOCALAPPDATA") {
        return Some(PathBuf::from(local).join("otter").join("cache"));
    }
    if let Some(xdg) = env::var_os("XDG_CACHE_HOME") {
        return Some(PathBuf::from(xdg).join("otter"));
    }
    if let Some(home) = env::var_os("HOME") {
        return Some(PathBuf::from(home).join(".cache").join("otter"));
    }
    Some(env::temp_dir().join("otter-cache"))
}

/// FNV-1a's offset basis and prime, applied by [`fnv1a`].
const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// Fold `bytes` into a running FNV-1a hash.
///
/// FNV-1a rather than [`std::collections::hash_map::DefaultHasher`] because the
/// cache key is written to the filesystem and read back by a *later* invocation
/// of the compiler: the hash has to mean the same thing across processes,
/// across Rust versions, and across machines sharing a cache directory.
/// `DefaultHasher`'s output is not promised to be stable, and a silent change
/// to it would orphan every cached object.
fn fnv1a(state: &mut u64, bytes: &[u8]) {
    for &byte in bytes {
        *state ^= u64::from(byte);
        *state = state.wrapping_mul(FNV_PRIME);
    }
}

/// The cache key for one runtime object: a hash of everything the object
/// depends on.
///
/// The runtime bytes are what actually make two objects differ, so they are the
/// core of the key and a changed runtime can never reuse an old object. The
/// triple and the optimization level change the code the toolchain would
/// produce. The tool's own path and modification time catch the one case the
/// source cannot: the same runtime compiled by a *different* `clang` is a
/// different object, and an in-place toolchain upgrade should not silently
/// reuse what the previous one emitted.
fn runtime_cache_key(tool: &Path, triple: &str, opt: OptLevel) -> u64 {
    let mut state = FNV_OFFSET_BASIS;
    fnv1a(&mut state, RUNTIME_C_SOURCE.as_bytes());
    fnv1a(&mut state, RUNTIME_HEADER.as_bytes());
    fnv1a(&mut state, triple.as_bytes());
    fnv1a(&mut state, opt.flag().as_bytes());
    fnv1a(&mut state, tool.to_string_lossy().as_bytes());
    if let Ok(modified) = fs::metadata(tool).and_then(|meta| meta.modified()) {
        if let Ok(since_epoch) = modified.duration_since(std::time::UNIX_EPOCH) {
            fnv1a(&mut state, &since_epoch.as_secs().to_le_bytes());
            fnv1a(&mut state, &since_epoch.subsec_nanos().to_le_bytes());
        }
    }
    state
}

/// The runtime, compiled to an object file that can be reused across builds.
///
/// Returns the object's path, leaving it in `cache_dir` for the next build to
/// find, or `None` when the cache cannot be used — an unwritable directory, a
/// toolchain that refuses to compile the runtime on its own. `None` is not an
/// error: the caller compiles the runtime source as part of the link, which is
/// slower and exactly as correct. A cache is only ever an optimization, so
/// nothing about it is allowed to fail a build.
///
/// The object is written to a staged name and renamed into place, so a build
/// interrupted mid-compile cannot leave a truncated object that the next build
/// would link as if it were complete. Two compilers racing to fill the same
/// entry both produce the same bytes; whichever renames second finds the
/// destination already there and keeps it.
fn cached_runtime_object(
    cache_dir: &Path,
    runtime_source: &Path,
    runtime_dir: &Path,
    target: &str,
    opt: OptLevel,
    toolchain: &Toolchain,
) -> Option<PathBuf> {
    let key = runtime_cache_key(&toolchain.llc, target, opt);
    let object = cache_dir.join(format!("otter-runtime-{key:016x}.o"));
    if object.is_file() {
        return Some(object);
    }
    fs::create_dir_all(cache_dir).ok()?;

    let staged = cache_dir.join(format!("otter-runtime-{key:016x}.{}.tmp", std::process::id()));
    let status = Command::new(&toolchain.llc)
        .args([
            "-c".to_string(),
            display(runtime_source),
            "-o".to_string(),
            display(&staged),
            format!("-I{}", display(runtime_dir)),
            format!("--target={target}"),
            opt.flag().to_string(),
        ])
        .output();
    match status {
        Ok(output) if output.status.success() => {}
        _ => {
            let _ = fs::remove_file(&staged);
            return None;
        }
    }

    if fs::rename(&staged, &object).is_err() {
        let _ = fs::remove_file(&staged);
        // Losing the rename means someone else filled the entry first, which
        // is just as good; only a genuinely absent object is a miss.
        if !object.is_file() {
            return None;
        }
    }
    Some(object)
}

/// Compile `source` to a native executable at `out`.
///
/// `entry_name` is the path reported in diagnostics and in the IR's
/// `source_filename`; it is what the user typed, not a canonicalised path.
///
/// `opt` is the `clang` optimization level for the generated code; see
/// [`OptLevel`] for why the caller, not codegen, chooses it.
///
/// `cpu` is the instruction set the generated code may use; see [`CpuTarget`]
/// for why the default is the portable baseline rather than this machine. It
/// applies to the IR and to nothing else — see the link step for why the
/// runtime deliberately stays portable.
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
    opt: OptLevel,
    cpu: CpuTarget,
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
    //
    // The optimization flag goes here, and again wherever the runtime C source
    // is compiled, so the program and its runtime are optimized alike. Without
    // it the runtime would stay at `-O0` while the program it is linked into
    // was optimized, which is the kind of asymmetry that shows up later as a
    // mystery in a profile.
    //
    // The *machine* flag is the opposite case: it goes here and nowhere else.
    // The runtime is a handful of `printf` wrappers called once at the end of a
    // program, so what `-march=native` buys it is unmeasurable, while keeping it
    // on the baseline is worth something real — one runtime object serves both
    // portable and native builds, so enabling `--native` does not throw away the
    // cache, and a cache shared between machines never hands one machine another
    // machine's instructions.
    let mut object_args: Vec<String> = vec![
        "-c".to_string(),
        display(&ir_path),
        "-o".to_string(),
        display(&object_path),
        format!("--target={target}"),
        opt.flag().to_string(),
    ];
    object_args.extend(cpu.flags().iter().map(|flag| flag.to_string()));
    run_tool(
        toolchain,
        &object_args,
        entry_name,
        "assembling the generated IR",
    )?;

    // 2. object + runtime -> executable.
    //
    // Reusing a cached runtime object removes the second compile of `runtime.c`
    // from the build entirely; the fallback hands the source to this same
    // invocation, which is why the optimization flag and include path appear in
    // only one of the two argument lists.
    let runtime_object = runtime_cache_dir().and_then(|cache_dir| {
        cached_runtime_object(
            &cache_dir,
            &runtime_source,
            &runtime_dir,
            target,
            opt,
            toolchain,
        )
    });
    let mut link_args = vec![display(&object_path)];
    match &runtime_object {
        Some(object) => link_args.push(display(object)),
        None => {
            link_args.push(display(&runtime_source));
            link_args.push(format!("-I{}", display(&runtime_dir)));
            link_args.push(opt.flag().to_string());
        }
    }
    link_args.push("-o".to_string());
    link_args.push(display(out));
    link_args.push(format!("--target={target}"));
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

#[cfg(test)]
mod tests {
    use super::{
        cached_runtime_object, fnv1a, runtime_cache_dir, runtime_cache_key, CpuTarget, OptLevel,
        FNV_OFFSET_BASIS,
    };
    use crate::toolchain::Toolchain;
    use std::fs;
    use std::path::{Path, PathBuf};

    fn fake_toolchain(tool: &Path) -> Toolchain {
        Toolchain {
            llc: tool.to_path_buf(),
            found: "test",
        }
    }

    #[test]
    fn the_default_level_optimizes() {
        // The default is a deliberate choice, not an accident of the derive:
        // passing no optimization level to `clang` means `-O0`.
        assert_eq!(OptLevel::default(), OptLevel::Full);
        assert_eq!(OptLevel::default().flag(), "-O2");
    }

    #[test]
    fn every_level_round_trips_through_its_flag() {
        for level in OptLevel::ALL {
            assert_eq!(OptLevel::parse(level.flag()), Some(level));
        }
    }

    #[test]
    fn the_default_target_is_portable() {
        // The default must not quietly emit this machine's instructions: the
        // value of the baseline is that the binary runs on a machine the
        // compiler has never seen.
        assert_eq!(CpuTarget::default(), CpuTarget::Portable);
        assert!(
            CpuTarget::default().flags().is_empty(),
            "a portable build must pass no machine flag at all"
        );
    }

    #[test]
    fn only_the_native_target_un_conserves_the_instruction_set() {
        assert_eq!(CpuTarget::Native.flags(), ["-march=native"]);
    }

    #[test]
    fn an_unrecognised_level_is_rejected_rather_than_clamped() {
        // `-O9`, `-O`, `--optimize` and a bare `-` are all usage errors; the
        // CLI turns `None` into a message rather than picking a level for the
        // user.
        for bad in ["-O9", "-O", "--optimize", "-", "-o2", "O2"] {
            assert_eq!(OptLevel::parse(bad), None, "`{bad}` should not parse");
        }
    }

    #[test]
    fn the_hash_notices_a_changed_byte_or_a_changed_order() {
        // The whole value of the content-addressed cache rests on this: a
        // runtime edited by one byte must not address the object built from its
        // predecessor, or the compiler would link a runtime it no longer has.
        let mut a = FNV_OFFSET_BASIS;
        fnv1a(&mut a, b"runtime");
        let mut same = FNV_OFFSET_BASIS;
        fnv1a(&mut same, b"runtime");
        let mut edited = FNV_OFFSET_BASIS;
        fnv1a(&mut edited, b"runtimf");
        let mut reordered = FNV_OFFSET_BASIS;
        fnv1a(&mut reordered, b"emittur");
        assert_eq!(a, same);
        assert_ne!(a, edited);
        assert_ne!(a, reordered);
    }

    #[test]
    fn the_cache_key_covers_everything_the_object_depends_on() {
        let tool = PathBuf::from("/toolchain/clang");
        let key = runtime_cache_key(&tool, "x86_64-pc-windows-gnu", OptLevel::Full);

        assert_eq!(
            key,
            runtime_cache_key(&tool, "x86_64-pc-windows-gnu", OptLevel::Full),
            "the same inputs must address the same object"
        );
        assert_ne!(
            key,
            runtime_cache_key(&tool, "x86_64-pc-windows-gnu", OptLevel::None),
            "a different optimization level is a different object"
        );
        assert_ne!(
            key,
            runtime_cache_key(&tool, "x86_64-unknown-linux-gnu", OptLevel::Full),
            "a different target is a different object"
        );
        assert_ne!(
            key,
            runtime_cache_key(
                &PathBuf::from("/other/clang"),
                "x86_64-pc-windows-gnu",
                OptLevel::Full
            ),
            "a different toolchain is a different object"
        );
    }

    #[test]
    fn there_is_always_somewhere_to_cache() {
        // Every branch of the lookup ends in a path, including the temp-dir
        // last resort, so "no cache directory" is never a reason a build skips
        // the cache.
        let dir = runtime_cache_dir().expect("a cache directory is always available");
        assert!(!dir.as_os_str().is_empty());
    }

    #[test]
    fn an_unusable_cache_directory_falls_back_instead_of_failing() {
        // A cache is an optimization: a directory that cannot be written must
        // produce `None` (compile the runtime at link time) and never an error.
        // A path that is an existing *file* cannot become a directory.
        let in_the_way = std::env::temp_dir().join("otter-cache-not-a-directory");
        let _ = fs::remove_dir_all(&in_the_way);
        fs::write(&in_the_way, b"in the way").expect("the test must be able to write a temp file");

        let object = cached_runtime_object(
            &in_the_way,
            &in_the_way.join("runtime.c"),
            &in_the_way,
            "x86_64-pc-windows-gnu",
            OptLevel::Full,
            &fake_toolchain(&in_the_way.join("clang")),
        );
        assert_eq!(object, None);

        let _ = fs::remove_file(&in_the_way);
    }
}
