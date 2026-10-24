//! The compilation pipeline: Yashy source in, native executable out.
//!
//! The stages run in one order and stop at the first failure:
//!
//! ```text
//! lex -> parse -> check -> emit IR -> clang (compile + link)
//! ```
//!
//! Nothing downstream of a failing stage runs, so a program with a syntax
//! error never reaches codegen and a program with a type error is never handed
//! to `clang`. This is what keeps a confusing linker message from ever being
//! the user's first feedback about a typo.
//!
//! # Where the runtime comes from
//!
//! The runtime is the *runtime C sources*, [`RUNTIME_C_SOURCE`] and
//! [`HASHMAP_C_SOURCE`], embedded in the compiler binary: there is no library
//! search path to configure and the runtime is always the one this compiler was
//! built and tested with.
//!
//! At link time the driver needs those sources as object files. Compiling them
//! costs a fraction of a build and their bytes never change between builds, so
//! each object is cached — see `cached_runtime_object`. The cache is keyed on
//! the exact bytes both were built from, so it cannot go stale: a changed
//! runtime is a different key and a different object. When the cache cannot be
//! used for any reason, the driver falls back to compiling the sources as part
//! of the link, which is what it did before there was a cache.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use yashy_codegen::{check_main_exists, emit_program, host_target_triple, CodegenOptions};
use yashy_common::{Diagnostic, FileId, Severity, SourceMap, Span};
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_runtime::{
    HASHMAP_C_SOURCE, HASHMAP_SOURCE_NAME, RUNTIME_C_SOURCE, RUNTIME_HEADER, RUNTIME_HEADER_NAME,
    RUNTIME_SOURCE_NAME,
};
use yashy_sema::{check_modules_with, check_program_with, CheckOptions, HirItem, HirProgram};

use crate::modules;
use crate::toolchain::Toolchain;

/// The `clang` optimization level applied to the generated IR.
///
/// The compiler emits unoptimized IR on purpose: the textual IR is a debugging
/// aid and staying close to the source is what makes it readable. All the
/// optimizing happens in LLVM, so "how fast is a Yashy binary" is decided here
/// rather than in `yashy-codegen`. The default is therefore `-O2`; a build that
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

/// Which instruction set the generated program may use.
///
/// The baseline of a `--target` triple is deliberately conservative: an
/// `x86_64` binary may only use what every x86-64 CPU has, so it runs anywhere.
/// That costs real performance — a loop the backend would otherwise vectorise
/// four integers at a time is held to two — but the alternative, emitting
/// instructions for the *build* machine, produces a binary that dies with an
/// illegal instruction on an older CPU.
///
/// There are two different ways to spend portability for speed here, and they
/// are not the same trade:
///
/// * a **feature level** (`x86-64-v2`, `v3`, `v4`) is a generation of the
///   architecture, defined by Intel and AMD and implemented by `clang`. A `v3`
///   binary needs an AVX2-era CPU — anything from 2013 on — but it is *the same
///   binary* on every such machine, so a build is reproducible and a `v3`
///   artifact can be shipped to someone else.
/// * [`CpuTarget::Native`] is this exact CPU. It is the fastest and the least
///   portable: the binary may name instructions no other machine has, and two
///   builds of one source can differ. It is for a program that will be built
///   and run on one machine.
///
/// Every one of them is opt-in. [`CpuTarget::Portable`] is the default because
/// the default has to run on a machine this compiler has never seen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CpuTarget {
    /// The triple's baseline: any CPU of that architecture can run the result.
    #[default]
    Portable,
    /// `-march=x86-64-v2`: SSE4.2 and friends, roughly 2009-era CPUs.
    X86_64V2,
    /// `-march=x86-64-v3`: AVX2, roughly 2013-era CPUs and later.
    X86_64V3,
    /// `-march=x86-64-v4`: AVX-512.
    X86_64V4,
    /// `-march=native`: the CPU this compiler is running on.
    Native,
}

impl CpuTarget {
    /// Every target, in the order a usage message should list them.
    pub const ALL: [CpuTarget; 5] = [
        CpuTarget::Portable,
        CpuTarget::X86_64V2,
        CpuTarget::X86_64V3,
        CpuTarget::X86_64V4,
        CpuTarget::Native,
    ];

    /// The extra `clang` arguments that select this target.
    ///
    /// Empty for the baseline, so a portable build passes nothing and behaves
    /// exactly as it did before this choice existed.
    pub fn flags(self) -> &'static [&'static str] {
        match self {
            CpuTarget::Portable => &[],
            CpuTarget::X86_64V2 => &["-march=x86-64-v2"],
            CpuTarget::X86_64V3 => &["-march=x86-64-v3"],
            CpuTarget::X86_64V4 => &["-march=x86-64-v4"],
            CpuTarget::Native => &["-march=native"],
        }
    }

    /// The spelling a user types, and the one a usage message lists.
    pub fn name(self) -> &'static str {
        match self {
            CpuTarget::Portable => "portable",
            CpuTarget::X86_64V2 => "x86-64-v2",
            CpuTarget::X86_64V3 => "x86-64-v3",
            CpuTarget::X86_64V4 => "x86-64-v4",
            CpuTarget::Native => "native",
        }
    }

    /// Parse a `--cpu` value. `None` for anything else, so the caller reports a
    /// usage error rather than guessing which tier was meant.
    pub fn parse(value: &str) -> Option<Self> {
        CpuTarget::ALL
            .into_iter()
            .find(|target| target.name() == value)
    }

    /// Whether this target names an x86-64 feature level.
    ///
    /// `x86-64-v2`/`v3`/`v4` are defined for x86-64 alone, so handing
    /// `-march=x86-64-v3` to a compiler that targets another architecture is not
    /// a conservative choice — it is a flag `clang` rejects. The CLI refuses the
    /// combination before a build starts. `Portable` and `Native` mean whatever
    /// the host architecture's baseline and current CPU are, and are always
    /// allowed.
    pub fn requires_x86_64(self) -> bool {
        matches!(
            self,
            CpuTarget::X86_64V2 | CpuTarget::X86_64V3 | CpuTarget::X86_64V4
        )
    }
}

/// Where compiled runtime objects are reused between builds.
///
/// `YASHY_CACHE_DIR` overrides the location, which is what a build system or a
/// test that wants a private cache sets. Otherwise it follows the platform's
/// convention: `%LOCALAPPDATA%` on Windows, `XDG_CACHE_HOME` or `~/.cache`
/// elsewhere. The temp directory is the last resort, because a cache that
/// cannot be written is only a lost optimization and not a failure — see
/// [`cached_runtime_object`].
fn runtime_cache_dir() -> Option<PathBuf> {
    if let Some(dir) = env::var_os("YASHY_CACHE_DIR") {
        if !dir.is_empty() {
            return Some(PathBuf::from(dir));
        }
    }
    if let Some(local) = env::var_os("LOCALAPPDATA") {
        return Some(PathBuf::from(local).join("yashy").join("cache"));
    }
    if let Some(xdg) = env::var_os("XDG_CACHE_HOME") {
        return Some(PathBuf::from(xdg).join("yashy"));
    }
    if let Some(home) = env::var_os("HOME") {
        return Some(PathBuf::from(home).join(".cache").join("yashy"));
    }
    Some(env::temp_dir().join("yashy-cache"))
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
    fnv1a(&mut state, HASHMAP_C_SOURCE.as_bytes());
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

/// One runtime source, compiled to an object file that can be reused across
/// builds.
///
/// Returns the object's path, leaving it in `cache_dir` for the next build to
/// find, or `None` when the cache cannot be used — an unwritable directory, a
/// toolchain that refuses to compile the source on its own. `None` is not an
/// error: the caller compiles the source as part of the link, which is slower
/// and exactly as correct. A cache is only ever an optimization, so nothing
/// about it is allowed to fail a build.
///
/// The runtime is two sources (`runtime.c` and `hashmap.c`) and the caller asks
/// for one object each, under a different `stem`. They share `cache_key`, which
/// hashes both sources, so the pair can never be half-stale.
///
/// The object is written to a staged name and renamed into place, so a build
/// interrupted mid-compile cannot leave a truncated object that the next build
/// would link as if it were complete. Two compilers racing to fill the same
/// entry both produce the same bytes; whichever renames second finds the
/// destination already there and keeps it.
fn cached_runtime_object(
    cache_dir: &Path,
    source: &Path,
    stem: &str,
    include_dir: &Path,
    cache_key: u64,
    target: &str,
    opt: OptLevel,
    toolchain: &Toolchain,
) -> Option<PathBuf> {
    let object = cache_dir.join(format!("yashy-{stem}-{cache_key:016x}.o"));
    if object.is_file() {
        return Some(object);
    }
    fs::create_dir_all(cache_dir).ok()?;

    let staged = cache_dir.join(format!(
        "yashy-{stem}-{cache_key:016x}.{}.tmp",
        std::process::id()
    ));
    let status = Command::new(&toolchain.llc)
        .args([
            "-c".to_string(),
            display(source),
            "-o".to_string(),
            display(&staged),
            format!("-I{}", display(include_dir)),
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

/// Everything a caller chooses about a build, apart from the source and the
/// output path.
///
/// Grouped rather than passed as separate positional arguments because all three
/// are optional choices with sensible defaults, and a call site that names them
/// reads better than one passing `OptLevel::default()` and `CpuTarget::default()`
/// positionally. [`BuildOptions::default`] is a portable, `-O2`,
/// exact-semantics build.
#[derive(Debug, Clone, Copy, Default)]
pub struct BuildOptions {
    /// The `clang` optimization level for the generated code. See [`OptLevel`].
    pub opt: OptLevel,
    /// The instruction set the generated code may use. See [`CpuTarget`].
    pub cpu: CpuTarget,
    /// Allow floating-point multiply-add contraction in the generated code; see
    /// [`CodegenOptions::fp_contract`]. It is only useful together with a
    /// target that has a fused multiply-add, which the portable baseline does
    /// not.
    pub fast_math: bool,
    /// A release build: `debug_assert` is not lowered. See
    /// [`yashy_sema::CheckOptions::release`].
    ///
    /// Independent of [`BuildOptions::opt`] on purpose. `-O2` is already the
    /// default, so a flag meaning "optimize harder" would be a no-op; this one
    /// answers a different question -- is this a build a user runs?
    pub release: bool,
}

/// The libraries this program asks for, as linker flags.
///
/// An `extern "C" link "m"` block is the source language's way of saying "this
/// binding lives in libm", and the driver is the only place that turns it into
/// a `-l` flag: the backend never sees a linker command line. Each library is
/// named once even when several blocks need it, in source order, so the command
/// a build failure quotes is the one a user wrote.
pub fn link_directives(hir: &HirProgram) -> Vec<String> {
    let mut flags: Vec<String> = Vec::new();
    for item in &hir.items {
        if let HirItem::Extern(block) = item {
            if let Some(library) = &block.link {
                let flag = format!("-l{library}");
                if !flags.contains(&flag) {
                    flags.push(flag);
                }
            }
        }
    }
    flags
}

/// The libraries a platform needs whether or not a program names them.
///
/// The runtime only uses stdio, but on Linux a link pulls in libm for a program
/// that calls `sqrt` and the thread runtime for one that does not use threads
/// at all; naming them costs nothing when unused. Windows folds all of this
/// into the CRT, which `clang` links by default, so there is nothing to add.
/// Every flag here is deduplicated against [`link_directives`], so `link "m"`
/// does not produce `-lm -lm`.
pub fn platform_library_flags() -> Vec<&'static str> {
    if cfg!(windows) {
        Vec::new()
    } else {
        vec!["-lpthread", "-ldl", "-lm"]
    }
}

/// The library named by a linker's "cannot find" message, if this output
/// contains one.
///
/// The two spellings are GNU `ld` (`cannot find -lfoo`) and LLD/Clang
/// (`unable to find library -lfoo`), which are the two linkers a Yashy build
/// can reach on a supported host. Naming the library turns "linking failed"
/// into a message that says which `link` directive to fix or install.
fn missing_library(stderr: &str) -> Option<String> {
    const MARKERS: [&str; 2] = ["unable to find library -l", "cannot find -l"];
    for line in stderr.lines() {
        for marker in MARKERS {
            let Some(index) = line.find(marker) else {
                continue;
            };
            let rest = line.get(index + marker.len()..).unwrap_or("");
            let name: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '+'))
                .collect();
            if !name.is_empty() {
                return Some(name);
            }
        }
    }
    None
}

/// Compile `source` to a native executable at `out`.
///
/// `entry_name` is the path reported in diagnostics and in the IR's
/// `source_filename`; it is what the user typed, not a canonicalised path.
///
/// `options` carries the three choices a caller makes about the result — the
/// optimization level, the instruction set, and whether floating-point
/// contraction is allowed. See [`BuildOptions`] for what each one means and why
/// all three default to the conservative, portable, exact-semantics setting.
///
/// Every diagnostic refers to file id **zero** of a fresh [`SourceMap`], which
/// is `entry_name`'s text. A caller that wants to render them builds its own
/// map by adding the same source first. This function does **not** resolve
/// imports: it is the one-file convenience, and a program that imports another
/// file should go through [`load_and_check`] and [`compile_hir`], the way the
/// CLI does.
///
/// Returns `Err` with every diagnostic of the first failing stage; on success,
/// `Ok(())` and no output at all (the caller decides what to print).
pub fn compile_to_binary(
    source: &str,
    entry_name: &str,
    out: &Path,
    toolchain: &Toolchain,
    options: BuildOptions,
) -> Result<(), Vec<Diagnostic>> {
    let mut sources = SourceMap::new();
    let file = sources.add_file(entry_name, source.to_string());

    let tokens = Lexer::with_file(source, file).tokenize()?;
    let mut parser = Parser::new(tokens, &sources, file);
    let program = parser.parse_program()?;
    let hir = check_program_with(
        &program,
        &sources,
        file,
        CheckOptions {
            release: options.release,
        },
    )?;

    compile_hir(&hir, entry_name, out, toolchain, options)
}

/// Load the entry point and its imports, then type-check the whole graph.
///
/// `sources` is filled with one file per module as the loader reads them. The
/// caller keeps it: every diagnostic names the module it belongs to, so
/// rendering needs the same map, not a new one built from the entry's text.
///
/// The entry point is file id zero. The returned [`HirProgram`] holds every
/// module's items in dependency order with the entry point last, which is the
/// order code generation emits them in.
pub fn load_and_check(
    sources: &mut SourceMap,
    entry_path: &Path,
    entry_source: String,
    release: bool,
) -> Result<HirProgram, Vec<Diagnostic>> {
    let modules = modules::load(sources, entry_path, entry_source)?;
    let programs: Vec<(FileId, &yashy_parser::ast::Program)> = modules
        .iter()
        .map(|module| (module.file, &module.program))
        .collect();
    check_modules_with(&programs, sources, CheckOptions { release })
}

/// Emit and link an already-checked program.
///
/// This is the half of the pipeline that needs an LLVM toolchain. Everything
/// above it is pure Rust, which is what lets `yashy check` and `--emit-ir` run
/// on a machine that cannot produce a binary.
pub fn compile_hir(
    hir: &HirProgram,
    entry_name: &str,
    out: &Path,
    toolchain: &Toolchain,
    options: BuildOptions,
) -> Result<(), Vec<Diagnostic>> {
    if let Err(diagnostic) = check_main_exists(&hir.items) {
        return Err(vec![diagnostic]);
    }

    let ir_text = emit_program(
        hir,
        entry_name,
        CodegenOptions {
            fp_contract: options.fast_math,
        },
    );

    // The IR next to the output is a debugging aid, not a temporary: when a
    // program misbehaves, "what did we actually hand to LLVM?" is the first
    // question, and it should not require re-running with a flag.
    let ir_path = with_extension(out, "ll");
    write_or_report(&ir_path, ir_text.as_bytes(), entry_name)?;

    let work_dir = out.parent().unwrap_or_else(|| Path::new(".")).to_path_buf();
    let runtime_dir = work_dir.join(format!(
        "{}.yashy",
        out.file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| "out".to_string())
    ));
    let runtime_source = runtime_dir.join(RUNTIME_SOURCE_NAME);
    let hashmap_source = runtime_dir.join(HASHMAP_SOURCE_NAME);
    ensure_dir(&runtime_dir, entry_name)?;
    write_or_report(&runtime_source, RUNTIME_C_SOURCE.as_bytes(), entry_name)?;
    write_or_report(&hashmap_source, HASHMAP_C_SOURCE.as_bytes(), entry_name)?;
    write_or_report(
        &runtime_dir.join(RUNTIME_HEADER_NAME),
        RUNTIME_HEADER.as_bytes(),
        entry_name,
    )?;

    let target = host_target_triple();

    // IR text + runtime -> executable, in **one** `clang` invocation.
    //
    // The obvious shape is two — `clang -c` the IR to an object, then link that
    // object with the runtime — and that is what this used to do. Two
    // invocations pay `clang`'s process start-up twice and write an intermediate
    // object that nothing reads, and the merged form is measurably faster for
    // byte-identical output: on the `fib` program, 15 interleaved runs put the
    // two-invocation shape at a 267 ms minimum and a 292 ms median, and the
    // merged one at 252 ms and 266 ms. See `docs/performance-improvements.md`.
    //
    // The optimization flag goes here, and again wherever the runtime C source
    // is compiled, so the program and its runtime are optimized alike. Without
    // it the runtime would stay at `-O0` while the program it is linked into
    // was optimized, which is the kind of asymmetry that shows up later as a
    // mystery in a profile.
    //
    // The machine flag is deliberately not applied to the runtime. The runtime
    // is a handful of `printf` wrappers called once at the end of a program, so
    // what `-march=native` or `-march=x86-64-v3` buys it is unmeasurable, while
    // keeping it on the baseline is worth something real: one cached runtime
    // object serves both portable and un-conserved builds, so choosing a target
    // does not throw the cache away, and a cache shared between machines never
    // hands one machine another machine's instructions. That is why the machine
    // flag reaches the compiler only when a cached object is available; on the
    // fallback path, where there is no cache to keep portable, the runtime is
    // compiled by this same invocation and takes the flag with it.
    let runtime_objects = runtime_cache_dir().and_then(|cache_dir| {
        let key = runtime_cache_key(&toolchain.llc, target, options.opt);
        let runtime = cached_runtime_object(
            &cache_dir,
            &runtime_source,
            "runtime",
            &runtime_dir,
            key,
            target,
            options.opt,
            toolchain,
        )?;
        let hashmap = cached_runtime_object(
            &cache_dir,
            &hashmap_source,
            "hashmap",
            &runtime_dir,
            key,
            target,
            options.opt,
            toolchain,
        )?;
        Some((runtime, hashmap))
    });

    let mut args: Vec<String> = vec![options.opt.flag().to_string(), format!("--target={target}")];
    args.extend(options.cpu.flags().iter().map(|flag| flag.to_string()));
    args.push(display(&ir_path));
    match &runtime_objects {
        Some((runtime, hashmap)) => {
            args.push(display(runtime));
            args.push(display(hashmap));
        }
        None => {
            args.push(display(&runtime_source));
            args.push(display(&hashmap_source));
            args.push(format!("-I{}", display(&runtime_dir)));
        }
    }
    args.push("-o".to_string());
    args.push(display(out));
    // Libraries last, after every object on the command line: GNU `ld` resolves
    // a symbol from the archives to its right, so `-lm` before the object that
    // calls `sqrt` would not find it. The `link` directives come first so a
    // failure quotes the user's intent before the platform defaults.
    let mut libraries = link_directives(hir);
    // On Windows the math functions live in the CRT: there is no separate `m`
    // library to name, and MSVC has no `m.lib` for `-lm` to resolve against.
    // Dropping the flag here is what lets one source file say `link "m"` for
    // Linux's sake and still link against the Windows CRT. GNU's `-lm` resolves
    // to a stub for the same CRT functions, so only the MSVC target needs this.
    // The directive is kept in [`link_directives`] unchanged, so diagnostics
    // still quote it.
    if cfg!(all(windows, target_env = "msvc")) {
        libraries.retain(|flag| flag != "-lm");
    }
    for flag in platform_library_flags() {
        let flag = flag.to_string();
        if !libraries.contains(&flag) {
            libraries.push(flag);
        }
    }
    args.extend(libraries);
    run_tool(toolchain, &args, entry_name, "compiling the program")?;

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
    if let Some(library) = missing_library(&stderr) {
        return Err(vec![toolchain_diagnostic(
            entry_name,
            format!(
                "`{}` failed while {what}: the linker cannot find the library `{library}`. \
                 A `link \"{library}\"` directive (or a platform default) asked for it; \
                 install it or link a library that exists.",
                toolchain.llc.display()
            ),
        )]);
    }
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
        cached_runtime_object, fnv1a, missing_library, platform_library_flags, runtime_cache_dir,
        runtime_cache_key, CpuTarget, OptLevel, FNV_OFFSET_BASIS,
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
    fn every_target_round_trips_through_its_name() {
        for target in CpuTarget::ALL {
            assert_eq!(CpuTarget::parse(target.name()), Some(target));
        }
    }

    #[test]
    fn every_non_portable_target_names_exactly_one_march_flag() {
        // The name a user types and the flag `clang` receives are the same
        // string, so a tier cannot be added to `ALL` and forgotten in `flags`.
        for target in CpuTarget::ALL {
            let flags = target.flags();
            if target == CpuTarget::Portable {
                assert!(flags.is_empty(), "the baseline passes no flag");
                continue;
            }
            assert_eq!(flags.len(), 1, "{target:?} should pass one flag");
            let expected = format!("-march={}", target.name());
            assert_eq!(flags[0], expected, "{target:?} must pass `{expected}`");
        }
    }

    #[test]
    fn an_unrecognised_cpu_target_is_rejected_rather_than_guessed() {
        // `--native` is a CLI alias, not a tier name, so the enum's parser must
        // not accept it: the mapping belongs with the other argument parsing.
        for bad in [
            "",
            "v3",
            "x86-64",
            "x86-64-v5",
            "arm64",
            "NATIVE",
            "--native",
        ] {
            assert_eq!(CpuTarget::parse(bad), None, "`{bad}` should not parse");
        }
    }

    #[test]
    fn only_the_feature_levels_are_marked_x86_only() {
        // `-march=x86-64-v3` is not a conservative flag on a non-x86 target, it
        // is one `clang` rejects, so the CLI has to know which targets it may
        // only offer on an x86-64 host.
        for target in CpuTarget::ALL {
            let expected = matches!(
                target,
                CpuTarget::X86_64V2 | CpuTarget::X86_64V3 | CpuTarget::X86_64V4
            );
            assert_eq!(target.requires_x86_64(), expected, "{target:?}");
        }
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
        let in_the_way = std::env::temp_dir().join("yashy-cache-not-a-directory");
        let _ = fs::remove_dir_all(&in_the_way);
        fs::write(&in_the_way, b"in the way").expect("the test must be able to write a temp file");

        let object = cached_runtime_object(
            &in_the_way,
            &in_the_way.join("runtime.c"),
            "runtime",
            &in_the_way,
            0,
            "x86_64-pc-windows-gnu",
            OptLevel::Full,
            &fake_toolchain(&in_the_way.join("clang")),
        );
        assert_eq!(object, None);

        let _ = fs::remove_file(&in_the_way);
    }

    #[test]
    fn a_missing_library_is_named_from_the_linker_message() {
        // The two linkers a Yashy build reaches spell it differently; both
        // messages have to turn into the library's name, not the whole line.
        assert_eq!(
            missing_library("ld.lld: error: unable to find library -lm"),
            Some("m".to_string())
        );
        assert_eq!(
            missing_library("/usr/bin/ld: cannot find -lfoo: No such file or directory"),
            Some("foo".to_string())
        );
        assert_eq!(
            missing_library("error: undefined reference to `sqrt'"),
            None,
            "an undefined symbol is not a missing library"
        );
        assert_eq!(
            missing_library("ld: cannot find -l\n"),
            None,
            "a truncated message must not claim an empty library"
        );
    }

    #[test]
    fn the_platform_defaults_cover_libm_and_are_empty_on_windows() {
        // sqrt needs libm on Linux and is in the CRT on Windows; `link \"m\"`
        // is deduplicated against these, so `-lm` must be here on the platform
        // that needs it and absent on the one that does not.
        let flags = platform_library_flags();
        if cfg!(windows) {
            assert!(flags.is_empty(), "the Windows CRT needs no flag: {flags:?}");
        } else {
            assert!(flags.contains(&"-lm"));
            assert!(flags.contains(&"-lpthread"));
            assert!(flags.contains(&"-ldl"));
        }
    }
}
