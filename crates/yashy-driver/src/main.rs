//! The `yashyc` command line interface.
//!
//! ```text
//! yashyc build <file.ya> [-o <out>] [--emit-ir] [--opt-report[=json]] [--stack-report[=json]] [--cpu <tier>] [--fast-math] [--release]
//! yashyc run <file.ya> [build flags] [-- <program args>...]
//! yashyc check <file.ya>
//! yashyc test [path] [--filter <substring>] [--list]
//! yashyc explain <code> [--json]
//! yashyc init [path]
//! yashyc doctor [--json]
//! yashyc expand <file.ya> [--json]
//! yashyc fmt [-w] <files...>
//! yashyc version
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
use std::process::{Command, ExitCode, ExitStatus, Stdio};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use yashy_codegen::{
    check_main_exists, emit_program, emit_program_with_notes, host_target_triple, promotion_counts,
    CodegenOptions,
};
use yashy_common::{
    codes, Diagnostic, FileId, Json, Severity, SourceMap, Span, EXIT_COMPILE_ERROR, EXIT_INTERNAL,
    EXIT_OK, EXIT_USAGE,
};
use yashy_driver::pipeline::{check_and_verify, compile_to_binary};
use yashy_driver::test_harness::{self, TestStatus};
use yashy_driver::{
    check_and_verify_timed, check_and_verify_timed_with_features, compile_hir_timed, detect,
    diagnostic_json, load_and_check_exports_timed, load_and_check_exports_timed_with_features,
    opt_report, resolve_opt, span_json, stack_report, timed, BuildOptions, CommandKind, CpuTarget,
    OptLevel, TimingRow,
};
use yashy_manifest::{Lockfile, Manifest};
use yashy_verify::{
    first_contract, Assumption, ContractMode, ProofObligation, VerifyOptions, VerifyReport,
};

/// How every command that verifies reads the program.
///
/// A contract is a promise the verifier checks, so an unproved one is an error
/// in every path that claims the program is sound: `verify`, `build`, `run` and
/// the human `test` path all share this one value. `yashyc test`'s JSON path
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
  yashyc build <file.ya> [-o <out>] [-O0|-O1|-O2|-O3] [--cpu <tier>] [--fast-math] [--release] [--no-lto] [--pgo] [--emit-ir] [--opt-report[=json]] [--timings] [--jobs <n>] [--no-opt=<pass>] [--opt-list] [--no-verify] [--stdlib-source] [--target <triple>] [--reproducible]
  yashyc run <file.ya> [-O0|-O1|-O2|-O3] [--cpu <tier>] [--fast-math] [--release] [--no-lto] [--timings] [--jobs <n>] [--no-opt=<pass>] [--no-verify] [--stdlib-source] [--target <triple>] [--reproducible] [--debug] [--sandbox] [--allow-read <path>] [--allow-write <path>] [--time <ms>] [--memory <mb>] [-- <args>...]
  yashyc tasks <file.ya> [-- <args>...]
  yashyc check <file.ya> [--json] [--timings] [--jobs <n>]
  yashyc lint <file.ya> [--json] [--list] [--group <g>] [--deny warnings] [--allow warnings] [--print-cfg] [--target <triple>]
  yashyc verify <file.ya> [--json] [--timings] [--jobs <n>] [--inventory]
  yashyc test [path] [--filter <substring>] [--list] [--json] [--timings] [--jobs <n>]
  yashyc inspect [--json] <file.ya> [symbol]
  yashyc cache verify <file.ya>
  yashyc serve [path]
  yashyc watch [path] [--json] [--run] [--test] [--interval <ms>]
  yashyc dev [path] [--json] [--interval <ms>]
  yashyc openapi <file.ya> [--title <name>] [--out <file>] [--check <old.json>]
  yashyc client --from <document.json> [--out <file.ya>]
  yashyc explain <code> [--json]
  yashyc init [path]
  yashyc doctor [--json]
  yashyc expand <file.ya> [--json]
  yashyc index [path] [--json]
  yashyc outline <file.ya>
  yashyc graph [path] [--json|--dot]
  yashyc search <query> [path] | yashyc search <query> --registry <dir>
  yashyc context --task <task> [path] [--budget <n>] [--json]
  yashyc mcp --root <dir>
  yashyc lsp [--version]
  yashyc doc [path] [--out DIR] [--json] [--private]
  yashyc profile <file.ya> [--sample] [--json] | yashyc profile --report <file.json>
  yashyc profile --alloc-report <file.ya> [--no-verify]
  yashyc lock --check
  yashyc fetch [--offline]
  yashyc update [--offline]
  yashyc add <name> --path <dir> | yashyc add <name> --git <url> --rev <commit>
  yashyc audit [dir] [--json]
  yashyc publish [dir] --registry <dir>
  yashyc search <query> --registry <dir>
  yashyc fmt [-w | --check] [--json] <files...>
  yashyc help [verb]
  yashyc completions <bash|zsh|fish|powershell>
  yashyc man
  yashyc version

  build   compile a program to a native executable
  run     compile and execute a program
  tasks   run a program and dump its live tasks (debug builds only)
  check   lex, parse and type-check without emitting anything
  verify  prove the program sound: every linear value has an owner
  test    compile and run the programs under tests/
  explain print the long-form page for an error code
  init    create a new Yashy project
  doctor  check the compiler's toolchain and standard library
  expand  print the fully expanded program, without checking it
  index   print the symbol index of a directory: signatures, effects and
          contracts (--json for the machine-readable form)
  outline print a file's symbols in source order
  graph   print the module and call graph (--json or Graphviz --dot)
  search  rank symbols and doc text by a query
  mcp     serve the seven compiler verbs over JSON-RPC on stdio
  lsp     run the language server (the sibling `yashy-lsp` binary)
  doc     render the public surface as Markdown (--json for the model)
  profile print a per-function counter report sorted by self time
  lock    check yashy.toml against yashy.lock without resolving anything
  fetch   put every package in the store without building
  update  resolve the manifests and rewrite yashy.lock
  add     add a path or pinned git dependency to yashy.toml, then update
  audit   inspect manifests, lockfile and package policy without building
  publish publish a package to a local directory registry
  search  search a local directory registry
  fmt     rewrite source files in the canonical style; -w writes them back
  -O<n>        LLVM optimization level for the generated code (default -O2)
  --cpu <tier> instruction set the program may use, one of:
               portable (default), x86-64-v2, x86-64-v3, x86-64-v4, native
               (`--native` is a synonym for `--cpu native`)
  --fast-math  allow fused multiply-add; faster float code, different results
  --release    a release build: `debug_assert` is compiled out and the
               optimization level defaults to -O3 unless an explicit -O
               level is given; a release build also links with ThinLTO
  --no-lto     link without ThinLTO; for bisecting a suspected LTO-only
               miscompile, normally unnecessary because LTO changes no result
  --pgo        build instrumented, run the program once, then rebuild with the
               profile; the data lives under `.yashy/pgo/` beside the output
  --emit-ir    print the generated LLVM IR to stdout and stop
  --opt-report print why every check and allocation survived, grouped by
               function, and stop; `--opt-report=json` prints the schema-v1
               form for an agent
  --stack-report print only the allocation placements from the same report,
               with the reason each site was promoted or left on the heap;
               `--stack-report=json` prints the schema-v1 form
  --no-verify  build without proving the program sound; refused when the program
               has a contract or `YASHY_REQUIRE_VERIFY` is set, and stamps
               `<output>.unverified` beside the binary
  --debug      with `run`: run the child with `YASHY_DEBUG=1`, so a panic or
               an arithmetic fault also prints every live task (id, function
               and resume index)
  --timings    print how long each pipeline phase took, to stderr (or in the
               JSON report); stdout is never touched
  --stdlib-source  derive the program from source instead of reusing cached IR;
               the cache is neither read nor written, for bisecting a build
  --jobs <n>   how many threads the front end may parse on; 0 (the default)
               uses one per available core, 1 parses serially
  --no-opt=<pass>  disable one registered optimization pass; `--opt-list` names
               them and the bench row each is held to. For bisecting only: none
               of them changes what a program prints
  --opt-list   list the optimization passes in order with their bench rows
  --inventory  with `verify`: list every `extern` declaration, `unsafe` call,
               and `unsafe` block, with the reason for each; never changes the exit code
  --certificates <file>  with `verify`: write one certificate per proof obligation,
               with its rule id, fact and source span; never changes the exit code
  --require-inventory  with `verify`: warn (E1404) when the program trusts C or
               unsafe code but no `--inventory` was asked for
  --color <auto|always|never>  when to colour output; auto is the default and
               degrades under NO_COLOR, CLICOLOR=0, TERM=dumb and a pipe
  --pager / --no-pager  page long human output through $PAGER on a terminal;
               a pipe never starts a pager, and `--json` never pages
  --hyperlink <auto|always|never>  make a diagnostic's `path:line:col` an OSC 8
               link; auto asks the terminal, and a pipe receives plain text
  --json       print a stable machine-readable report (check, verify, test, explain, doctor)";

/// The stack the front end runs on; the parser owns the requirement because
/// [`yashy_parser::MAX_NESTING_DEPTH`] is only meaningful on a stack this large.
const FRONT_END_STACK_BYTES: usize = yashy_parser::FRONT_END_STACK_BYTES;

fn main() -> ExitCode {
    let mut args: Vec<String> = env::args().skip(1).collect();
    args = match yashy_driver::term::take_global_flags(args) {
        Ok(args) => args,
        Err(message) => {
            eprintln!("error: {message}");
            return exit_code(EXIT_USAGE);
        }
    };
    let worker_args = args.clone();
    match std::thread::Builder::new()
        .name("yashyc".to_string())
        .stack_size(FRONT_END_STACK_BYTES)
        .spawn(move || dispatch(&worker_args))
    {
        Ok(worker) => worker.join().unwrap_or_else(|_| exit_code(EXIT_INTERNAL)),
        // If the thread cannot be created the command still runs, on whatever
        // stack the process already has.
        Err(_) => dispatch(&args),
    }
}

fn dispatch(args: &[String]) -> ExitCode {
    let Some(command) = args.first().map(String::as_str) else {
        eprintln!("error: no command given\n{USAGE}");
        return exit_code(EXIT_USAGE);
    };
    let rest = args.get(1..).unwrap_or(&[]);

    match command {
        "version" | "--version" | "-V" => {
            println!("yashyc {}", env!("CARGO_PKG_VERSION"));
            exit_code(EXIT_OK)
        }
        "help" | "--help" | "-h" => help_cmd(rest),
        "completions" => completions_cmd(rest),
        "man" => {
            print!("{}", yashy_driver::cli_spec::render_man());
            exit_code(EXIT_OK)
        }
        "build" => build(CommandKind::Build, rest),
        "run" => run(rest),
        "tasks" => tasks(rest),
        "check" => check(rest),
        "lint" => lint_cmd(rest),
        "coverage" => coverage_cmd(rest),
        "fix" => fix_cmd(rest),
        "migrate" => migrate_cmd(rest),
        "verify" => verify(rest),
        "test" => test(rest),
        "bench" => bench_cmd(rest),
        "fuzz" => fuzz_cmd(rest),
        "mutate" => mutate_cmd(rest),
        "cache" => cache_cmd(rest),
        "inspect" => inspect_cmd(rest),
        "tokens" => tokens(rest),
        "trace" => trace_cmd(rest),
        "serve" => serve(rest),
        "watch" => watch_cmd(rest),
        "dev" => dev_cmd(rest),
        "openapi" => openapi_cmd(rest),
        "record" => record_cmd(rest),
        "replay" => replay_cmd(rest),
        "client" => client_cmd(rest),
        "explain" => explain(rest),
        "init" => init(rest),
        "new" => new_cmd(rest),
        "repl" => repl_cmd(rest),
        "debug" => debug_cmd(rest),
        "task" => task_cmd(rest),
        "doctor" => doctor(rest),
        "expand" => expand(rest),
        "index" => index_cmd(rest),
        "outline" => outline_cmd(rest),
        "graph" => graph_cmd(rest),
        "search" => search_cmd(rest),
        "context" => context_cmd(rest),
        "review" => review_cmd(rest),
        "impact" => impact_cmd(rest),
        "conformance" => conformance_cmd(rest),
        "advisory" => advisory_cmd(rest),
        "mcp" => mcp_cmd(rest),
        "agent" => agent_cmd(rest),
        "lsp" => lsp_cmd(rest),
        "doc" => doc_cmd(rest),
        "profile" => profile_cmd(rest),
        "lock" => lock_cmd(rest),
        "fetch" => fetch_cmd(rest),
        "update" => update_cmd(rest),
        "add" => add_cmd(rest),
        "audit" => audit_cmd(rest),
        "sbom" => sbom_cmd(rest),
        "verify-package" => verify_package_cmd(rest),
        "self-update" => self_update_cmd(rest),
        "attest" => attest_cmd(rest),
        "tree" => tree_cmd(rest),
        "why" => why_cmd(rest),
        "info" => info_cmd(rest),
        "outdated" => outdated_cmd(rest),
        "vendor" => vendor_cmd(rest),
        "remove" => remove_cmd(rest),
        "yank" => yank_cmd(rest),
        "verify-attestation" => verify_attestation_cmd(rest),
        "publish" => publish_cmd(rest),
        "fmt" => fmt(rest),
        "playground" => playground(rest),
        other => {
            match yashy_driver::cli_spec::closest(other) {
                Some(near) => eprintln!("error: unknown command `{other}`; did you mean `{near}`?"),
                None => eprintln!("error: unknown command `{other}`"),
            }
            exit_code(EXIT_USAGE)
        }
    }
}

/// `yashyc help [verb]`: the full table, or one verb's flags.
fn help_cmd(args: &[String]) -> ExitCode {
    if args.is_empty() {
        print!("{}", yashy_driver::cli_spec::render_help());
        return exit_code(EXIT_OK);
    }
    if args.len() > 1 {
        return usage_error("`help` takes at most one verb");
    }
    let name = &args[0];
    match yashy_driver::cli_spec::render_verb(name) {
        Some(text) => {
            print!("{text}");
            exit_code(EXIT_OK)
        }
        None => {
            eprintln!("error: no help for unknown verb `{name}`");
            exit_code(EXIT_USAGE)
        }
    }
}

/// `yashyc completions <shell>`: one generated script per shell.
fn completions_cmd(args: &[String]) -> ExitCode {
    let Some(shell) = args.first() else {
        return usage_error("`completions` needs a shell: bash, zsh, fish or powershell");
    };
    if args.len() > 1 {
        return usage_error("`completions` takes one shell");
    }
    match yashy_driver::cli_spec::render_completions(shell) {
        Ok(script) => {
            print!("{script}");
            exit_code(EXIT_OK)
        }
        Err(error) => {
            eprintln!("error: {error}");
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
    timings: Vec<TimingRow>,
}

/// The feature set a source file's project selected, or a usage error.
///
/// `--features` and `--no-default-features` name a selection the manifest must
/// define. A project with no `[features]` table and no request selects the
/// empty set, which is what every build before features existed meant; asking
/// for a feature a project does not declare is a mistake, not a no-op.
fn select_features(
    input: Option<&Path>,
    requested: &[String],
    no_default_features: bool,
) -> Result<std::collections::BTreeSet<String>, ExitCode> {
    let manifest_path = input
        .and_then(Path::parent)
        .unwrap_or_else(|| Path::new("."))
        .join("yashy.toml");
    match Manifest::load(&manifest_path) {
        Ok(manifest) => {
            match yashy_driver::features::select(&manifest, requested, no_default_features) {
                Ok(features) => Ok(features.enabled),
                Err(message) => Err(usage_error(&message)),
            }
        }
        Err(_) if requested.is_empty() && !no_default_features => {
            Ok(std::collections::BTreeSet::new())
        }
        Err(_) => Err(usage_error(
            "`--features` needs a yashy.toml with a `[features]` table",
        )),
    }
}

/// `yashyc build <file.ya> [-o <out>] [-O0|-1|-O2|-O3] [--cpu <tier>] [--fast-math] [--release] [--emit-ir] [--timings] [--no-verify]`.
fn build(command: CommandKind, args: &[String]) -> ExitCode {
    let outcome = build_measured(command, args);
    // An empty vector means the flag was not passed, so this is silent then.
    report_timings(&outcome.timings);
    outcome.code
}

fn build_measured(command: CommandKind, args: &[String]) -> BuildOutcome {
    let mut input: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    let mut emit_ir = false;
    let mut timings_flag = false;
    let mut no_verify = false;
    let mut deny_warnings = env::var("YASHY_DENY_WARNINGS").ok().as_deref() == Some("1");
    let mut allow_warnings = false;
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
    // ThinLTO is the release link, and `--no-lto` takes it away so a suspected
    // LTO-only miscompile can be attributed rather than worked around.
    let mut no_lto = false;
    // Profile-guided builds: instrument, train, rebuild with the profile.
    let mut pgo = false;
    // How many threads the front end may parse on. `0` means "one per core".
    let mut jobs: usize = 0;
    let mut requested_features: Vec<String> = Vec::new();
    let mut no_default_features = false;

    // `--offline` never touches the network or invokes git. A package that is
    // not already in the store is `E1803`, never a silent fetch.
    let mut offline = false;
    // `--stdlib-source` derives the stdlib from source even when a matching
    // IR cache entry exists: it neither reads nor writes the cache. It is the
    // escape hatch for bisecting a suspected cache problem.
    let mut stdlib_source = false;
    // Registered optimization passes the user asked to disable. Validated
    // after parsing, so one message can list every pass that exists.
    let mut disabled: Vec<String> = Vec::new();
    // Print the registry and stop; no input is needed.
    let mut opt_list = false;
    // `--opt-report` prints why decisions survived and stops. `None` is "not
    // asked for"; the `bool` is whether the JSON form was requested.
    let mut opt_report_flag: Option<bool> = None;
    // `--stack-report` selects only the placement rows of the same record.
    let mut stack_report_flag: Option<bool> = None;
    // The triple this build is for. `None` means the host, which is what every
    // build before `--target` existed meant.
    let mut target: Option<&'static str> = None;
    // Build for bytes that depend only on the tree, not on the clock or the
    // build directory.
    let mut reproducible = false;
    // Emit LLVM debug metadata from HIR spans. Off unless a debug build asks
    // for a line table.
    let mut debug_info = false;
    let mut print_capabilities = false;

    let mut args = args.iter();
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--print-capabilities" => print_capabilities = true,
            "--emit-ir" => emit_ir = true,
            "--timings" => timings_flag = true,
            "--debug-info" => debug_info = true,
            "--features" => match args.next() {
                Some(value) => requested_features.extend(value.split(',').map(str::to_string)),
                None => return stopped(usage_error("`--features` needs a comma-separated list")),
            },
            "--no-default-features" => no_default_features = true,
            "--no-verify" => no_verify = true,
            "--jobs" => match args.next() {
                Some(value) => match value.parse::<usize>() {
                    Ok(parsed) => jobs = parsed,
                    Err(_) => return stopped(usage_error("`--jobs` needs a whole number")),
                },
                None => return stopped(usage_error("`--jobs` needs a value")),
            },
            "--deny" => match args.next() {
                Some(value) if value == "warnings" => deny_warnings = true,
                _ => return stopped(usage_error("unknown option `--deny` for `build`")),
            },
            "--allow" => match args.next() {
                Some(value) if value == "warnings" => allow_warnings = true,
                _ => return stopped(usage_error("unknown option `--allow` for `build`")),
            },
            "-W" => match args.next() {
                Some(value) if value == "none" => allow_warnings = true,
                _ => return stopped(usage_error("unknown option `-W` for `build`")),
            },
            "-W none" => allow_warnings = true,
            "--fast-math" => fast_math = true,
            "--reproducible" => reproducible = true,
            "--target" => match args.next() {
                Some(value) => match target_flag(value) {
                    Ok(triple) => target = Some(triple),
                    Err(code) => return stopped(code),
                },
                None => return stopped(usage_error("`--target` needs a triple")),
            },
            "--offline" => offline = true,
            "--stdlib-source" => stdlib_source = true,
            "--opt-list" => opt_list = true,
            "--opt-report" => opt_report_flag = Some(false),
            "--stack-report" => stack_report_flag = Some(false),
            "--release" => release = true,
            "--no-lto" => no_lto = true,
            "--pgo" => pgo = true,
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
                } else if let Some(value) = other.strip_prefix("--jobs=") {
                    match value.parse::<usize>() {
                        Ok(parsed) => jobs = parsed,
                        Err(_) => return stopped(usage_error("`--jobs` needs a whole number")),
                    }
                } else if let Some(value) = other.strip_prefix("--target=") {
                    match target_flag(value) {
                        Ok(triple) => target = Some(triple),
                        Err(code) => return stopped(code),
                    }
                } else if let Some(pass) = other.strip_prefix("--no-opt=") {
                    disabled.push(pass.to_string());
                } else if let Some(value) = other.strip_prefix("--opt-report=") {
                    match value {
                        "json" => opt_report_flag = Some(true),
                        _ => {
                            return stopped(usage_error(
                                "`--opt-report` takes no value, or `=json`",
                            ))
                        }
                    }
                } else if let Some(value) = other.strip_prefix("--stack-report=") {
                    match value {
                        "json" => stack_report_flag = Some(true),
                        _ => {
                            return stopped(usage_error(
                                "`--stack-report` takes no value, or `=json`",
                            ))
                        }
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

    // A pass name is validated once, here, against the one registry: an
    // unknown name is a usage error that names every name that works.
    for pass in &disabled {
        if yashy_codegen::pass::find(pass).is_none() {
            return stopped(usage_error(&format!(
                "`{pass}` is not an optimization pass; valid passes: {} (see --opt-list)",
                yashy_codegen::pass::names()
            )));
        }
    }
    let escape = !disabled.iter().any(|pass| pass == "escape");
    let facts = !disabled.iter().any(|pass| pass == "facts");
    let intern = !disabled.iter().any(|pass| pass == "intern");
    let async_ = !disabled.iter().any(|pass| pass == "async");

    if opt_list {
        println!("{:<8} {}", "pass", "row");
        for pass in yashy_codegen::pass::pipeline() {
            println!("{:<8} {}", pass.name, pass.row);
        }
        return stopped(exit_code(EXIT_OK));
    }

    // One value from here on: the IR header, the data layout, the `cfg`
    // filter, the platform libraries and the `clang --target` line all read
    // it, so a build cannot be for two machines at once.
    let target = target.unwrap_or_else(host_target_triple);

    // A feature level is an x86-64 name and nothing else. On any other target it
    // is not a conservative choice, it is a flag `clang` rejects, so the
    // mismatch is refused here where it can name the two architectures rather
    // than left for `clang` to report against a build far from the cause.
    if cpu.requires_x86_64() && !target.starts_with("x86_64") {
        return stopped(usage_error(&format!(
            "`{}` is an x86-64 feature level, but this build is for `{target}`",
            cpu.name()
        )));
    }

    if input.is_none() {
        // A workspace root builds its default member when no file is named.
        if let Ok(Some(found)) = yashy_driver::workspace::load(Path::new(".")) {
            if let Ok(members) = found.select(None, false, &[]) {
                if let Some(member) = members.first() {
                    input = Some(member.join("main.ya"));
                }
            }
        }
    }
    let Some(input) = input else {
        return stopped(usage_error("`build` needs a source file"));
    };

    // A project that carries both a manifest and a lockfile has made a
    // reproducibility claim. Verify it before compiling; that is the point of
    // the lock. A manifest without a lock is not an error yet: the first lock
    // is written by `yashyc update` once resolution has a source to pin.
    if let Some(code) = verify_project_lock(&input) {
        return stopped(code);
    }

    // Resolve the project's packages and make their module roots visible to the
    // import loader. A build with no manifest, or one with no dependencies, is
    // exactly what it was before packages existed.
    let package_roots = match prepare_packages(&input, offline) {
        Ok(roots) => roots,
        Err(error) => {
            error.report();
            return stopped(error.status());
        }
    };
    if !package_roots.is_empty() {
        expose_package_roots(&input, &package_roots);
    }

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
    if pgo && emit_ir {
        return stopped(usage_error(
            "`--pgo` needs a binary to train; it cannot be combined with `--emit-ir`",
        ));
    }
    if opt_report_flag.is_some() {
        if emit_ir {
            return stopped(usage_error(
                "`--opt-report` and `--emit-ir` both stop before linking; choose one",
            ));
        }
        if no_verify {
            return stopped(usage_error(
                "`--opt-report` explains what the verifier proved; it cannot be combined \
                 with `--no-verify`",
            ));
        }
        if pgo {
            return stopped(usage_error(
                "`--pgo` needs a binary to train; it cannot be combined with `--opt-report`",
            ));
        }
    }

    if stack_report_flag.is_some() {
        if emit_ir || opt_report_flag.is_some() {
            return stopped(usage_error(
                "`--stack-report`, `--opt-report` and `--emit-ir` each stop before \
                 linking; choose one",
            ));
        }
        if no_verify {
            return stopped(usage_error(
                "`--stack-report` explains what the verifier proved; it cannot be \
                 combined with `--no-verify`",
            ));
        }
        if pgo {
            return stopped(usage_error(
                "`--pgo` needs a binary to train; it cannot be combined with \
                 `--stack-report`",
            ));
        }
    }

    let source = match read_source(&input) {
        Ok(source) => source,
        Err(code) => return stopped(code),
    };
    let entry = if reproducible {
        reproducible_entry(&input)
    } else {
        input.to_string_lossy().into_owned()
    };

    // Timings are collected only when asked. An empty vector is how the
    // outcome says "the flag was not passed", so `build` prints without having
    // to parse the arguments a second time.
    let mut timings = Vec::new();
    let mut scratch = Vec::new();
    let measured: &mut Vec<TimingRow> = if timings_flag {
        &mut timings
    } else {
        &mut scratch
    };

    // One map for the whole program: the loader adds every imported file to it
    // as it reads that file, so a diagnostic from a module renders against the
    // module's own text. The entry point is file id zero.
    let mut sources = SourceMap::new();

    // Features are decided before anything is parsed, because a `cfg` filter
    // reads them: `--features tls` must decide whether `cfg(feature:tls)` items
    // exist in the tree the checker sees, not merely be recorded afterwards.
    let selected_features =
        match select_features(Some(&input), &requested_features, no_default_features) {
            Ok(features) => features,
            Err(code) => return stopped(code),
        };

    let (hir, report) = if no_verify {
        // The checker still runs: `--no-verify` skips the proof, not the type
        // system. A program that does not type-check has no binary worth
        // stamping.
        let hir = match load_and_check_exports_timed_with_features(
            &mut sources,
            &input,
            source,
            release,
            target,
            jobs,
            &selected_features,
            measured,
        ) {
            Ok((hir, _exports)) => hir,
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
        match check_and_verify_timed_with_features(
            &mut sources,
            &input,
            source,
            release,
            target,
            &OPTIONS,
            jobs,
            &selected_features,
            measured,
        ) {
            Ok(checked) => checked,
            Err(diagnostics) => {
                return BuildOutcome {
                    code: render(&sources, &diagnostics),
                    timings,
                }
            }
        }
    };

    let (hir, mut report) = (hir, report);

    // A manifest that declares capabilities asks for call-site enforcement.
    // Older projects with no `[capabilities]` table keep their existing build.
    let manifest_path = input
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("yashy.toml");
    let manifest = Manifest::load(&manifest_path).ok();
    if print_capabilities {
        if let Some(manifest) = &manifest {
            let resolved = yashy_driver::policy::resolve(manifest, &[]);
            let mut names: Vec<String> = resolved
                .granted
                .entries()
                .iter()
                .map(|(name, _)| (*name).to_string())
                .collect();
            names.extend(resolved.denied);
            names.sort();
            names.dedup();
            println!("capabilities: {}", names.join(", "));
        } else {
            println!("capabilities: none");
        }
        return BuildOutcome {
            code: exit_code(EXIT_OK),
            timings,
        };
    }
    if let Some(manifest) = &manifest {
        let capability_diagnostics = yashy_driver::capability_check::check(&hir, manifest);
        if !capability_diagnostics.is_empty() {
            let mut local = SourceMap::new();
            local.add_file(
                input.to_string_lossy(),
                fs::read_to_string(&input).unwrap_or_default(),
            );
            return BuildOutcome {
                code: render(&local, &capability_diagnostics),
                timings,
            };
        }
    }

    // Apply the warning policy: --allow suppresses, --deny upgrades to errors.
    if allow_warnings {
        report
            .diagnostics
            .retain(|d| d.severity != Severity::Warning);
    } else if deny_warnings {
        for d in &mut report.diagnostics {
            if d.severity == Severity::Warning {
                d.severity = Severity::Error;
            }
        }
    }

    // An unverified binary is not a build. A verifier diagnostic may be a
    // warning, which is printed and does not stop the build; an error stops it
    // here, before `clang` is ever started.
    if !report.is_empty() {
        let code = render(&sources, &report.diagnostics);
        if report.error_count() > 0 {
            return BuildOutcome { code, timings };
        }
    }

    // `--stack-report` is the allocation-only view of the report the emitter
    // already produced. It stops before the linker like `--opt-report`.
    if let Some(as_json) = stack_report_flag {
        let code = match check_main_exists(&hir.items) {
            Ok(()) => {
                let options = CodegenOptions {
                    capabilities: None,
                    fp_contract: fast_math,
                    escape,
                    facts,
                    intern,
                    async_,
                    target,
                };
                let (_, notes) = timed(measured, "emit", || {
                    emit_program_with_notes(&hir, &entry, options, &report.facts)
                });
                if as_json {
                    let document = stack_report::json(&sources, &notes);
                    println!("{}", document.render());
                } else {
                    print!("{}", stack_report::render(&sources, &notes));
                }
                exit_code(EXIT_OK)
            }
            Err(diagnostic) => render(&sources, &[diagnostic]),
        };
        return BuildOutcome { code, timings };
    }

    // `--opt-report` is a front-end operation like `--emit-ir`: it explains the
    // decisions of one emission and stops, so it needs no linker.
    if let Some(as_json) = opt_report_flag {
        let code = match check_main_exists(&hir.items) {
            Ok(()) => {
                let options = CodegenOptions {
                    capabilities: None,
                    fp_contract: fast_math,
                    escape,
                    facts,
                    intern,
                    async_,
                    target,
                };
                let (_, notes) = timed(measured, "emit", || {
                    emit_program_with_notes(&hir, &entry, options, &report.facts)
                });
                if as_json {
                    let document = opt_report::json(&sources, &notes);
                    println!("{}", document.render());
                } else {
                    print!("{}", opt_report::render(&sources, &notes));
                }
                exit_code(EXIT_OK)
            }
            Err(diagnostic) => render(&sources, &[diagnostic]),
        };
        return BuildOutcome { code, timings };
    }

    // A manifest that declares capabilities installs the same resolved grants
    // at run time that the HIR call-site check enforced at build time. Computed
    // before the front-end-only verbs so `--emit-ir` shows the same IR a build
    // would compile.
    let capabilities = manifest.as_ref().and_then(|manifest| {
        let resolved = yashy_driver::policy::resolve(manifest, &[]);
        let mut names: Vec<String> = resolved
            .granted
            .entries()
            .iter()
            .map(|(name, _)| (*name).to_string())
            .collect();
        names.sort();
        (!names.is_empty()).then(|| names.join(","))
    });
    // `--emit-ir` is a pure front-end operation, so it deliberately does not
    // require an LLVM toolchain: dumping IR is exactly what you want to do on a
    // machine where the backend cannot run yet.
    if emit_ir {
        let code = match check_main_exists(&hir.items) {
            Ok(()) => {
                let options = CodegenOptions {
                    capabilities: capabilities.clone(),
                    fp_contract: fast_math,
                    escape,
                    facts,
                    intern,
                    async_,
                    target,
                };
                let ir = timed(measured, "emit", || {
                    if debug_info {
                        // `--emit-ir --debug-info` is the debug IR a user can
                        // read and a test can assert on, with no toolchain in
                        // the loop: the same markers a debug build emits,
                        // turned into the same metadata.
                        let text = yashy_codegen::emit_program_with_debug(
                            &hir,
                            &entry,
                            options,
                            &report.facts,
                        );
                        yashy_driver::debug_info::annotate(&text, &hir, &sources)
                    } else {
                        emit_program(&hir, &entry, options, &report.facts)
                    }
                });
                if escape {
                    let (stack, arena) = promotion_counts(&hir, &report.facts);
                    measured.push(TimingRow::Promotion { stack, arena });
                }
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

    // A cross build links with the target's C toolchain, which the compiler
    // neither ships nor can find by name. Asking once here, before the front
    // end runs, is the difference between one line naming the missing
    // toolchain and a link error after a full compile. `--emit-ir` returned
    // above, so a machine without a cross compiler can still inspect the IR.
    if target != host_target_triple() {
        if let Err(message) = yashy_driver::target::probe_cross(&toolchain, target) {
            let looked_for = format!("looked for `{}`", toolchain.llc.display());
            return BuildOutcome {
                code: coded_error(codes::TARGET_TOOLCHAIN_MISSING, &message, &[looked_for]),
                timings,
            };
        }
    }

    let output = output.unwrap_or_else(|| default_output(&input));
    let opt = resolve_opt(command, release, opt);
    let options = BuildOptions {
        capabilities,
        features: selected_features,
        debug_info,
        opt,
        cpu,
        fast_math,
        escape,
        facts,
        intern,
        release,
        lto: release && !no_lto,
        pgo,
        async_,
        jobs,
        ir_cache: !stdlib_source,
        target,
        reproducible,
    };
    let code = match compile_hir_timed(
        &sources,
        &hir,
        &entry,
        &output,
        &toolchain,
        options,
        &report.facts,
        measured,
    ) {
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

/// The name the IR carries for the entry file in a reproducible build.
///
/// The name has to be the same in two builds of the same source in two
/// directories, so it is relative to the working directory when the file is
/// under it, and the file's own name when it is not. A path that named the
/// build directory would make the IR -- and every hash computed from it --
/// depend on where the build happened.
fn reproducible_entry(path: &Path) -> String {
    let relative = std::env::current_dir()
        .ok()
        .and_then(|root| path.strip_prefix(root).ok())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| {
            path.file_name()
                .map(PathBuf::from)
                .unwrap_or_else(|| path.to_path_buf())
        });
    relative.to_string_lossy().replace('\\', "/")
}

/// `yashyc playground [--port <n>] [--host <address>]`.
///
/// The playground compiles on the server: every run is a sandboxed
/// `yashyc run`, with a directory of its own and the default-deny capability
/// set. The static front end is served from `playground/`.
fn playground(args: &[String]) -> ExitCode {
    let mut port: u16 = 8080;
    let mut host = "127.0.0.1".to_string();
    let mut iter = args.iter();
    while let Some(argument) = iter.next() {
        match argument.as_str() {
            "--port" => {
                let Some(value) = iter.next() else {
                    return usage_error("`--port` needs a number");
                };
                match value.parse::<u16>() {
                    Ok(parsed) => port = parsed,
                    Err(_) => return usage_error("`--port` needs a number"),
                }
            }
            "--host" => {
                let Some(value) = iter.next() else {
                    return usage_error("`--host` needs an address");
                };
                host = value.clone();
            }
            other => return usage_error(&format!("unknown option `{other}` for `playground`")),
        }
    }

    let listener = match std::net::TcpListener::bind((host.as_str(), port)) {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("error: cannot listen on {host}:{port}: {error}");
            return exit_code(EXIT_INTERNAL);
        }
    };
    let address = listener
        .local_addr()
        .map(|address| address.to_string())
        .unwrap_or_else(|_| format!("{host}:{port}"));
    println!("playground: http://{address}");
    let mut server =
        yashy_driver::playground::Server::new(yashy_driver::playground::Limits::default());
    if let Err(error) = yashy_driver::playground::serve(&mut server, listener) {
        eprintln!("error: the playground stopped: {error}");
        return exit_code(EXIT_INTERNAL);
    }
    exit_code(EXIT_OK)
}

/// The provenance stamp that sits beside an unverified binary.///
/// `hello.exe` becomes `hello.exe.unverified`, so the pair travels together
/// through a copy or a package without a manifest to keep in step.
fn unverified_sidecar(output: &Path) -> PathBuf {
    let mut path = output.as_os_str().to_owned();
    path.push(".unverified");
    PathBuf::from(path)
}

/// The stamp's exact bytes. One line of JSON, so a reader can parse it without
/// guessing at a format, and a human can read it with `cat`. The version is
/// the compiler's, taken from the manifest so the stamp cannot drift from
/// `yashyc --version`.
const UNVERIFIED_SIDECAR: &str = concat!(
    r#"{"schema_version":1,"verified":false,"reason":"--no-verify","yashy_version":""#,
    env!("CARGO_PKG_VERSION"),
    r#""}"#
);

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

/// `yashyc run <file.ya> [build flags] [-- <program args>...]`.
/// `yashyc run --debug <file.ya> ...` is the same run with the child's
/// `YASHY_DEBUG` set: a panic or arithmetic fault also prints the executor's
/// live tasks. `yashyc tasks <file.ya>` goes one step further (see
/// [`RunMode::Tasks`]): it is the task table for whatever the program does.
///
/// The output path is temporary on purpose: `run` is for the edit/run loop,
/// and a user who wants to keep the binary has `build -o`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum RunMode {
    /// `yashyc run`: no debug environment.
    Plain,
    /// `yashyc run --debug`: the child gets `YASHY_DEBUG=1`.
    Debug,
    /// `yashyc tasks`: a debug build only. The child gets `YASHY_DEBUG=1`
    /// and `YASHY_TASKS_DUMP=1`, so the task table prints on a panic and
    /// while the run spins on a pending task — the async analogue of a
    /// thread dump.
    Tasks,
}

fn run(args: &[String]) -> ExitCode {
    run_as(args, RunMode::Plain)
}

/// `yashyc repl`: a compiling REPL over the real checker and compiler.
///
/// Declarations are kept as source text and rechecked with every line. An
/// expression becomes a one-function program that prints its value; its type
/// is read from the HIR the checker produced. There is no second interpreter.
/// `yashyc debug <file.ya> [--break <fn>]... [-- <args>...]`
///
/// Builds a program, writes a debugger script and starts the native backend.
/// Breakpoints are on symbol names: the backend does not emit a line table yet,
/// so `docs/debugging.md` says so instead of promising source lines.
/// `yashyc trace [--json] [--fail-orphans] [root]`
///
/// The static rule-to-code-to-test matrix. Runtime `--events` tracing is not
/// implemented; asking for it says so instead of pretending.
/// Build a debug binary, run it with the runtime's event trace enabled, and
/// render the recorded events.
///
/// The runtime records raw return addresses; this command turns them into
/// source spans with `llvm-symbolizer`. The binary is built in a scratch
/// directory and removed afterwards, so `trace --events` leaves no artifact
/// behind except the report it prints.
fn trace_events_cmd(source: &Path, program_args: &[String], json: bool) -> ExitCode {
    use yashy_driver::trace;

    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    let scratch =
        std::env::temp_dir().join(format!("yashy-trace-events-{}-{stamp}", std::process::id()));
    if let Err(error) = fs::create_dir_all(&scratch) {
        eprintln!("error: cannot create {}: {error}", scratch.display());
        return exit_code(EXIT_INTERNAL);
    }
    let binary = scratch.join(format!("program{}", std::env::consts::EXE_SUFFIX));
    let events_file = scratch.join("events.txt");
    let build_args = vec![
        source.to_string_lossy().into_owned(),
        "-o".to_string(),
        binary.to_string_lossy().into_owned(),
        "--debug-info".to_string(),
    ];
    let built = build(CommandKind::Build, &build_args);
    let ok = exit_code(EXIT_OK);
    let result = if built != ok {
        built
    } else {
        let mut child = Command::new(&binary);
        child
            .env("YASHY_TRACE_FILE", &events_file)
            .stdout(Stdio::null());
        for argument in program_args {
            child.arg(argument);
        }
        match child.status() {
            Ok(status) if status.success() => match fs::read_to_string(&events_file) {
                Ok(text) => {
                    let (mut events, dropped) = trace::parse_events(&text);
                    trace::symbolize(&mut events, &binary);
                    if json {
                        println!("{}", trace::events_json(&events, dropped, source).render());
                    } else {
                        for event in &events {
                            println!(
                                "{:>4} {:<11} {:<32} {}",
                                event.seq, event.kind, event.span, event.size
                            );
                        }
                        if dropped > 0 {
                            eprintln!(
                                "warning: {dropped} event(s) dropped; the trace table is full"
                            );
                        }
                    }
                    exit_code(EXIT_OK)
                }
                Err(error) => {
                    eprintln!("error: cannot read {}: {error}", events_file.display());
                    exit_code(EXIT_INTERNAL)
                }
            },
            Ok(status) => {
                eprintln!("warning: the traced program exited with {status}");
                exit_code(EXIT_OK)
            }
            Err(error) => {
                eprintln!("error: cannot run {}: {error}", binary.display());
                exit_code(EXIT_INTERNAL)
            }
        }
    };
    let _ = fs::remove_dir_all(&scratch);
    result
}

/// `yashyc inspect --rc <file.ya>`: build and run the program with the
/// runtime's reference-count table dumped at exit, then render the live cells.
/// A cell still held at exit is named as a leak or a cycle; the runtime cannot
/// tell the two apart without an object graph, and the report says so.
fn inspect_rc_cmd(source: &Path, json: bool) -> ExitCode {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    let scratch =
        std::env::temp_dir().join(format!("yashy-inspect-rc-{}-{stamp}", std::process::id()));
    if let Err(error) = fs::create_dir_all(&scratch) {
        eprintln!("error: cannot create {}: {error}", scratch.display());
        return exit_code(EXIT_INTERNAL);
    }
    let binary = scratch.join(format!("program{}", std::env::consts::EXE_SUFFIX));
    let cells_file = scratch.join("rc.txt");
    let build_args = vec![
        source.to_string_lossy().into_owned(),
        "-o".to_string(),
        binary.to_string_lossy().into_owned(),
    ];
    let built = build(CommandKind::Build, &build_args);
    let ok = exit_code(EXIT_OK);
    let result = if built != ok {
        built
    } else {
        let mut child = Command::new(&binary);
        child.env("YASHY_RC_FILE", &cells_file);
        match child.status() {
            Ok(status) if status.success() => match fs::read_to_string(&cells_file) {
                Ok(text) => {
                    let mut cells = Vec::new();
                    for line in text.lines() {
                        let fields: Vec<&str> = line.split_whitespace().collect();
                        if fields.len() != 3 {
                            continue;
                        }
                        let (Ok(handle), Ok(strong), Ok(weak)) = (
                            fields[0].parse::<i64>(),
                            fields[1].parse::<i64>(),
                            fields[2].parse::<i64>(),
                        ) else {
                            continue;
                        };
                        cells.push((handle, strong, weak));
                    }
                    if json {
                        let rows: Vec<Json> = cells
                            .iter()
                            .map(|(handle, strong, weak)| {
                                let mut row = Json::object();
                                row.set("handle", Json::number(*handle));
                                row.set("strong", Json::number(*strong));
                                row.set("weak", Json::number(*weak));
                                row.set("live_at_exit", Json::bool(true));
                                row
                            })
                            .collect();
                        let mut report = Json::object();
                        report.set("schema_version", Json::number(1));
                        report.set("tool", Json::string("inspect"));
                        report.set("mode", Json::string("rc"));
                        report.set(
                            "source",
                            Json::string(source.to_string_lossy().into_owned()),
                        );
                        report.set("cells", Json::array(rows));
                        println!("{}", report.render());
                    } else {
                        println!("handle strong weak");
                        for (handle, strong, weak) in &cells {
                            println!("{handle:>6} {strong:>6} {weak:>4}");
                        }
                        if cells.is_empty() {
                            println!("no live reference-counted cells");
                        } else {
                            eprintln!(
                                "note: a live strong count is a leak or a cycle; \
                                 the runtime cannot tell which without an object graph"
                            );
                        }
                    }
                    exit_code(EXIT_OK)
                }
                Err(error) => {
                    eprintln!("error: cannot read {}: {error}", cells_file.display());
                    exit_code(EXIT_INTERNAL)
                }
            },
            Ok(status) => {
                eprintln!("warning: the inspected program exited with {status}");
                exit_code(EXIT_OK)
            }
            Err(error) => {
                eprintln!("error: cannot run {}: {error}", binary.display());
                exit_code(EXIT_INTERNAL)
            }
        }
    };
    let _ = fs::remove_dir_all(&scratch);
    result
}

/// `yashyc bench [--json] [--list] [file]`.
///
/// Reads the `key=value` rows a benchmark script wrote and renders them. It
/// deliberately measures nothing: a claim without a row is not a claim this
/// repository accepts (see `docs/bench-policy.md`).
fn bench_cmd(args: &[String]) -> ExitCode {
    use yashy_driver::bench;

    let mut json = false;
    let mut list = true;
    let mut path: Option<PathBuf> = None;
    let mut baseline: Option<PathBuf> = None;
    let mut thresholds: Option<PathBuf> = None;
    let mut measure_source: Option<PathBuf> = None;
    let mut compare_binary: Option<PathBuf> = None;
    let mut save_baseline: Option<PathBuf> = None;
    let mut key = "bench".to_string();
    let mut iterations = 5usize;
    let mut warmup = 1usize;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--json" => json = true,
            "--list" => list = true,
            "--check-baseline" => {
                if i + 1 >= args.len() {
                    return usage_error("`--check-baseline` needs a file path");
                }
                baseline = Some(PathBuf::from(&args[i + 1]));
                i += 1;
            }
            "--thresholds" => {
                if i + 1 >= args.len() {
                    return usage_error("`--thresholds` needs a file path");
                }
                thresholds = Some(PathBuf::from(&args[i + 1]));
                i += 1;
            }
            "--measure" => {
                if i + 1 >= args.len() {
                    return usage_error("`--measure` needs a source file");
                }
                measure_source = Some(PathBuf::from(&args[i + 1]));
                i += 1;
            }
            "--compare" => {
                if i + 1 >= args.len() {
                    return usage_error("`--compare` needs a binary path");
                }
                compare_binary = Some(PathBuf::from(&args[i + 1]));
                i += 1;
            }
            "--save-baseline" => {
                if i + 1 >= args.len() {
                    return usage_error("`--save-baseline` needs a file path");
                }
                save_baseline = Some(PathBuf::from(&args[i + 1]));
                i += 1;
            }
            "--key" => {
                if i + 1 >= args.len() {
                    return usage_error("`--key` needs a name");
                }
                key = args[i + 1].clone();
                i += 1;
            }
            "--iterations" => {
                if i + 1 >= args.len() {
                    return usage_error("`--iterations` needs a number");
                }
                match args[i + 1].parse::<usize>() {
                    Ok(value) if value > 0 => iterations = value,
                    _ => return usage_error("`--iterations` needs a positive number"),
                }
                i += 1;
            }
            "--warmup" => {
                if i + 1 >= args.len() {
                    return usage_error("`--warmup` needs a number");
                }
                match args[i + 1].parse::<usize>() {
                    Ok(value) => warmup = value,
                    Err(_) => return usage_error("`--warmup` needs a number"),
                }
                i += 1;
            }
            other if other.starts_with('-') => {
                return usage_error(&format!("unknown option `{other}` for `bench`"));
            }
            other => {
                if path.is_some() {
                    return usage_error("`bench` takes one row file");
                }
                path = Some(PathBuf::from(other));
            }
        }
        i += 1;
    }
    if measure_source.is_none() {
        if save_baseline.is_some() {
            return usage_error("`--save-baseline` needs `--measure`");
        }
        if compare_binary.is_some() {
            return usage_error("`--compare` needs `--measure`");
        }
    }
    if let Some(source) = measure_source {
        use yashy_driver::bench_measure::{measure, baseline_text, to_json, MeasureOptions};
        let options = MeasureOptions {
            source,
            compare: compare_binary,
            iterations,
            warmup,
            key,
        };
        let result = match measure(&options) {
            Ok(result) => result,
            Err(error) => {
                eprintln!("error: benchmark failed: {error}");
                return exit_code(EXIT_USAGE);
            }
        };
        if let Some(out) = save_baseline {
            if let Err(error) = std::fs::write(&out, baseline_text(&result)) {
                eprintln!("error: cannot write {}: {error}", out.display());
                return exit_code(EXIT_USAGE);
            }
        }
        if json {
            println!("{}", to_json(&result));
        } else {
            println!(
                "{} median {:.3} ms p95 {:.3} ms ({} samples, {} discarded)",
                result.key, result.median_ms, result.p95_ms, result.samples, result.discarded
            );
        }
        if let Some(other) = result.compare_median_ms {
            println!("compare median {other:.3} ms");
        }
        return exit_code(EXIT_OK);
    }
    let path = path.unwrap_or_else(|| PathBuf::from("benchmarks/thresholds.bench"));
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) => {
            eprintln!("error: cannot read {}: {error}", path.display());
            return exit_code(EXIT_USAGE);
        }
    };
    let rows = bench::parse(&text);
    if let Some(baseline_path) = baseline {
        let base_text = match std::fs::read_to_string(&baseline_path) {
            Ok(text) => text,
            Err(error) => {
                eprintln!("error: cannot read {}: {error}", baseline_path.display());
                return exit_code(EXIT_USAGE);
            }
        };
        let threshold_rows = match thresholds {
            Some(path) => match std::fs::read_to_string(&path) {
                Ok(text) => bench::parse(&text),
                Err(error) => {
                    eprintln!("error: cannot read {}: {error}", path.display());
                    return exit_code(EXIT_USAGE);
                }
            },
            None => bench::parse(&text),
        };
        let diffs = bench::compare(&rows, &bench::parse(&base_text), &threshold_rows);
        let regressed = diffs.iter().any(|diff| diff.regressed);
        if json {
            println!("{}", bench::compare_json(&diffs).render());
        } else {
            print!("{}", bench::render_comparison(&diffs));
        }
        return if regressed {
            exit_code(EXIT_COMPILE_ERROR)
        } else {
            exit_code(EXIT_OK)
        };
    }
    if json {
        println!("{}", bench::to_json(&rows).render());
    } else if list {
        for row in &rows {
            println!("{}={}", row.key, row.value);
        }
    }
    exit_code(EXIT_OK)
}

/// `yashyc fuzz [--json] [root]`.
///
/// Lists the corpus entries the repository keeps. It does not run the fuzzer:
/// that target is a plain function in `packages/fuzz`, and a finding is a file
/// whose bytes can be replayed. An empty corpus is an honest answer.
fn fuzz_cmd(args: &[String]) -> ExitCode {
    use yashy_driver::fuzz;

    let mut json = false;
    let mut findings: Option<PathBuf> = None;
    let mut replay: Option<PathBuf> = None;
    let mut root: Option<PathBuf> = None;
    let mut run_target: Option<PathBuf> = None;
    let mut iterations = 200usize;
    let mut seed = 1i64;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--json" => json = true,
            "--findings" => {
                if i + 1 >= args.len() {
                    return usage_error("`--findings` needs a file path");
                }
                findings = Some(PathBuf::from(&args[i + 1]));
                i += 1;
            }
            "--replay" => {
                if i + 1 >= args.len() {
                    return usage_error("`--replay` needs a finding path");
                }
                replay = Some(PathBuf::from(&args[i + 1]));
                i += 1;
            }
            "--run" => {
                if i + 1 >= args.len() {
                    return usage_error("`--run` needs a target source file");
                }
                run_target = Some(PathBuf::from(&args[i + 1]));
                i += 1;
            }
            "--iterations" => {
                if i + 1 >= args.len() {
                    return usage_error("`--iterations` needs a number");
                }
                match args[i + 1].parse::<usize>() {
                    Ok(value) if value > 0 => iterations = value,
                    _ => return usage_error("`--iterations` needs a positive number"),
                }
                i += 1;
            }
            "--seed" => {
                if i + 1 >= args.len() {
                    return usage_error("`--seed` needs a number");
                }
                match args[i + 1].parse::<i64>() {
                    Ok(value) => seed = value,
                    Err(_) => return usage_error("`--seed` needs a number"),
                }
                i += 1;
            }
            other if other.starts_with('-') => {
                return usage_error(&format!("unknown option `{other}` for `fuzz`"));
            }
            other => {
                if root.is_some() {
                    return usage_error("`fuzz` takes one root directory");
                }
                root = Some(PathBuf::from(other));
            }
        }
        i += 1;
    }
    if let Some(target_path) = run_target {
        let source = match std::fs::read_to_string(&target_path) {
            Ok(text) => text,
            Err(error) => {
                eprintln!("error: cannot read {}: {error}", target_path.display());
                return exit_code(EXIT_USAGE);
            }
        };
        let mut replay_input: Option<String> = None;
        if let Some(path) = replay.as_ref() {
            let text = match std::fs::read_to_string(path) {
                Ok(text) => text,
                Err(error) => {
                    eprintln!("error: cannot read {}: {error}", path.display());
                    return exit_code(EXIT_USAGE);
                }
            };
            for token in text.split_whitespace() {
                if let Some(value) = token.strip_prefix("seed=") {
                    if let Ok(parsed) = value.parse::<i64>() {
                        seed = parsed;
                    }
                }
                if let Some(value) = token.strip_prefix("iterations=") {
                    if let Ok(parsed) = value.parse::<usize>() {
                        iterations = parsed;
                    }
                }
            }
            replay_input = text
                .split_whitespace()
                .find_map(|token| token.strip_prefix("input=").map(|value| value.to_string()));
            if replay_input.is_none() {
                eprintln!("error: finding has no `input=` field");
                return exit_code(EXIT_USAGE);
            }
        }
        let dir = std::env::temp_dir().join(format!("yashy-fuzz-{}", std::process::id()));
        let app_dir = dir.join("app");
        let target_dir = dir.join("targetsrc");
        if let Err(error) = std::fs::create_dir_all(&app_dir) {
            eprintln!("error: cannot create {}: {error}", app_dir.display());
            return exit_code(EXIT_INTERNAL);
        }
        if let Err(error) = std::fs::create_dir_all(&target_dir) {
            eprintln!("error: cannot create {}: {error}", target_dir.display());
            return exit_code(EXIT_INTERNAL);
        }
        if let Err(error) = std::fs::write(target_dir.join("target.ya"), &source) {
            eprintln!("error: cannot write target: {error}");
            return exit_code(EXIT_INTERNAL);
        }
        let driver = format!(
            "import campaign;\nimport target;\nfn accepts_input(string value) -> bool {{\n    return target.accepts(value);\n}}\nfn main() {{\n if let Some(f) = campaign.campaign(accepts_input, {seed}, {iterations}, 4, 12) {{\n  print(campaign.describe(f));\n }}\n}}\n"
        );
        if let Err(error) = std::fs::write(app_dir.join("main.ya"), driver) {
            eprintln!("error: cannot write campaign driver: {error}");
            return exit_code(EXIT_INTERNAL);
        }
        let compiler = match std::env::current_exe() {
            Ok(path) => path,
            Err(_) => return exit_code(EXIT_INTERNAL),
        };
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let paths = format!(
            "{}{}{}{}{}",
            root.join("crates/yashy-stdlib/yashy").display(),
            if cfg!(windows) { ";" } else { ":" },
            root.join("packages/fuzz/src").display(),
            if cfg!(windows) { ";" } else { ":" },
            target_dir.display()
        );
        let binary = dir.join(format!("program{}", std::env::consts::EXE_SUFFIX));
        let built = Command::new(&compiler)
            .arg("build")
            .arg(app_dir.join("main.ya"))
            .arg("-o")
            .arg(&binary)
            .env("YASHY_PATH", &paths)
            .current_dir(&root)
            .status()
            .map(|status| status.success())
            .unwrap_or(false);
        if !built {
            return exit_code(EXIT_COMPILE_ERROR);
        }
        let run = Command::new(&binary)
            .output()
            .expect("the campaign must run");
        let output = String::from_utf8_lossy(&run.stdout).trim().to_string();
        let produced_input = output
            .split_whitespace()
            .find_map(|token| token.strip_prefix("input=").map(|value| value.to_string()));
        if let Some(expected) = replay_input {
            match produced_input {
                Some(actual) if actual == expected => {
                    println!("replay ok: {output}");
                    return exit_code(EXIT_OK);
                }
                Some(actual) => {
                    eprintln!("error: replay diverged: expected `{expected}`, got `{actual}`");
                    return exit_code(EXIT_COMPILE_ERROR);
                }
                None => {
                    eprintln!("error: replay did not reproduce a finding");
                    return exit_code(EXIT_COMPILE_ERROR);
                }
            }
        }
        if output.starts_with("seed=") {
            let record = format!("seed={seed}\niterations={iterations}\n{output}\n");
            let path = match findings {
                Some(path) => path,
                None => {
                    let dir = PathBuf::from("fuzz/findings");
                    if let Err(error) = std::fs::create_dir_all(&dir) {
                        eprintln!("error: cannot create {}: {error}", dir.display());
                        return exit_code(EXIT_INTERNAL);
                    }
                    let hash = yashy_driver::record::program_hash(&record);
                    dir.join(format!("{hash}.bin"))
                }
            };
            if let Err(error) = std::fs::write(&path, &record) {
                eprintln!("error: cannot write finding: {error}");
                return exit_code(EXIT_USAGE);
            }
            println!("finding: {} ({output})", path.display());
            return exit_code(EXIT_COMPILE_ERROR);
        }
        if json {
            println!("{{\"schema_version\":1,\"tool\":\"fuzz\",\"iterations\":{iterations},\"finding\":null}}");
        } else {
            println!("fuzz: no finding in {iterations} iterations");
        }
        return exit_code(EXIT_OK);
    }
    if let Some(path) = replay {
        match std::fs::read(&path) {
            Ok(bytes) => {
                println!("{}", String::from_utf8_lossy(&bytes));
                return exit_code(EXIT_OK);
            }
            Err(error) => {
                eprintln!("error: cannot read {}: {error}", path.display());
                return exit_code(EXIT_USAGE);
            }
        }
    }
    let root = root.unwrap_or_else(|| PathBuf::from("fuzz/corpus"));
    let entries = match fuzz::list(&root) {
        Ok(entries) => entries,
        Err(message) => {
            eprintln!("error: {message}");
            return exit_code(EXIT_INTERNAL);
        }
    };
    if let Some(path) = findings {
        match std::fs::write(&path, fuzz::findings_json(&entries).render()) {
            Ok(()) => {}
            Err(error) => {
                eprintln!("error: cannot write {}: {error}", path.display());
                return exit_code(EXIT_USAGE);
            }
        }
    }
    if json {
        println!("{}", fuzz::to_json(&entries).render());
    } else {
        for entry in &entries {
            println!("{} {}", entry.bytes, entry.path);
        }
    }
    exit_code(EXIT_OK)
}

/// `yashyc mutate --list`.
///
/// Lists the AST-level mutation operators with stable names. The mutator
/// itself lives in `yashy-mutate`; this verb is the inventory, not a campaign
/// runner. A score without a named operator is not a score.
fn mutate_cmd(args: &[String]) -> ExitCode {
    if let Some(at) = args.iter().position(|arg| arg == "--score") {
        let Some(value) = args.get(at + 1) else {
            return usage_error("`--score` needs killed:survived:timed_out:equivalent");
        };
        let parts: Vec<&str> = value.split(':').collect();
        if parts.len() != 4 || parts.iter().any(|part| part.parse::<usize>().is_err()) {
            return usage_error("`--score` needs four whole numbers separated by `:`");
        }
        let count = |index: usize| parts[index].parse::<usize>().unwrap_or(0);
        let report = yashy_mutate::MutationReport {
            killed: count(0),
            survived: count(1),
            timed_out: count(2),
            equivalent: count(3),
        };
        let score = report.score();
        if args.iter().any(|arg| arg == "--json") || !args.iter().any(|arg| arg == "--fail-under") {
            println!("{}", report.to_json().render());
        } else {
            println!("mutation score: {score:.2}%");
        }
        if let Some(at) = args.iter().position(|arg| arg == "--fail-under") {
            let Some(value) = args.get(at + 1) else {
                return usage_error("`--fail-under` needs a percentage");
            };
            match value.parse::<f64>() {
                Ok(floor) if score >= floor => return exit_code(EXIT_OK),
                Ok(_) => return exit_code(EXIT_COMPILE_ERROR),
                Err(_) => return usage_error("`--fail-under` needs a percentage"),
            }
        }
        return exit_code(EXIT_OK);
    }
    if args.iter().any(|arg| arg == "--json") {
        let mut report = Json::object();
        report.set("schema_version", Json::number(1));
        report.set("tool", Json::string("mutate"));
        let entries: Vec<Json> = yashy_mutate::OPERATORS
            .iter()
            .map(|(name, description)| {
                let mut object = Json::object();
                object.set("name", Json::string(*name));
                object.set("description", Json::string(*description));
                object
            })
            .collect();
        report.set("operators", Json::array(entries));
        println!("{}", report.render());
        return exit_code(EXIT_OK);
    }
    if args.is_empty() || args.iter().any(|arg| arg == "--list") {
        for (name, description) in yashy_mutate::OPERATORS {
            println!("{name}: {description}");
        }
        return exit_code(EXIT_OK);
    }
    for argument in args {
        if argument.starts_with('-') {
            return usage_error(&format!("unknown option `{argument}` for `mutate`"));
        }
    }
    usage_error("`mutate` takes `--list` or `--json`")
}

fn trace_cmd(args: &[String]) -> ExitCode {
    use yashy_driver::trace;

    let mut json = false;
    let mut markdown = false;
    let mut fail_orphans = false;
    let mut events = false;
    let mut program_args: Vec<String> = Vec::new();
    let mut after_separator = false;
    let mut root: Option<PathBuf> = None;
    for argument in args {
        if after_separator {
            program_args.push(argument.clone());
            continue;
        }
        match argument.as_str() {
            "--json" => json = true,
            "--markdown" => markdown = true,
            "--fail-orphans" => fail_orphans = true,
            "--events" => events = true,
            "--" => after_separator = true,
            other if other.starts_with('-') => {
                return usage_error(&format!("unknown option `{other}` for `trace`"));
            }
            other => {
                if root.is_some() {
                    return usage_error("`trace` takes one root directory");
                }
                root = Some(PathBuf::from(other));
            }
        }
    }
    if events {
        let Some(source) = root else {
            return usage_error("`trace --events` needs a source file");
        };
        return trace_events_cmd(&source, &program_args, json);
    }
    let root = root.unwrap_or_else(|| PathBuf::from("."));
    let matrix = match trace::matrix(&root) {
        Ok(matrix) => matrix,
        Err(message) => {
            eprintln!("error: {message}");
            return exit_code(EXIT_INTERNAL);
        }
    };
    let rows = &matrix.rows;
    let orphans: Vec<&trace::RuleRow> = rows.iter().filter(|row| row.tests.is_empty()).collect();
    let orphan_codes = &matrix.orphan_codes;
    let unenforced_rules = &matrix.orphan_rules_without_code;
    let orphan_stdlib = &matrix.orphan_stdlib_functions;
    let orphan_cases = &matrix.orphan_conformance_cases;
    if json {
        println!("{}", trace::to_json(&matrix).render());
    } else if markdown {
        print!("{}", trace::to_markdown(&rows));
    } else {
        for row in rows {
            println!(
                "{:<14} {} code(s), {} test file(s)",
                row.rule,
                row.codes.len(),
                row.tests.len()
            );
        }
    }
    if fail_orphans
        && (!orphans.is_empty()
            || !orphan_codes.is_empty()
            || !unenforced_rules.is_empty()
            || !orphan_stdlib.is_empty()
            || !orphan_cases.is_empty())
    {
        if !orphans.is_empty() {
            eprintln!(
                "error: {} rule(s) have no test: {}",
                orphans.len(),
                orphans
                    .iter()
                    .map(|row| row.rule.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        if !orphan_codes.is_empty() {
            eprintln!(
                "error: {} code(s) have no test: {}",
                orphan_codes.len(),
                orphan_codes.join(", ")
            );
        }
        if !unenforced_rules.is_empty() {
            eprintln!(
                "error: {} rule(s) have no enforcing code: {}",
                unenforced_rules.len(),
                unenforced_rules.join(", ")
            );
        }
        if !orphan_stdlib.is_empty() {
            eprintln!(
                "error: {} stdlib function(s) have no program: {}",
                orphan_stdlib.len(),
                orphan_stdlib.join(", ")
            );
        }
        if !orphan_cases.is_empty() {
            eprintln!(
                "error: {} conformance case(s) name an unenforced rule: {}",
                orphan_cases.len(),
                orphan_cases.join(", ")
            );
        }
        return exit_code(EXIT_COMPILE_ERROR);
    }
    exit_code(EXIT_OK)
}

fn debug_cmd(args: &[String]) -> ExitCode {
    use yashy_driver::debug;

    let mut breakpoints: Vec<String> = Vec::new();
    let mut binary: Option<PathBuf> = None;
    let mut operands: Vec<String> = Vec::new();
    let mut program_args: Vec<String> = Vec::new();
    let mut after_separator = false;
    let mut iter = args.iter();
    while let Some(argument) = iter.next() {
        if after_separator {
            program_args.push(argument.clone());
            continue;
        }
        match argument.as_str() {
            "--" => after_separator = true,
            "--break" => {
                let Some(value) = iter.next() else {
                    return usage_error("`--break` needs a function name");
                };
                breakpoints.push(value.clone());
            }
            "--binary" => {
                let Some(value) = iter.next() else {
                    return usage_error("`--binary` needs a path");
                };
                binary = Some(PathBuf::from(value));
            }
            other if other.starts_with('-') => {
                return usage_error(&format!("unknown option `{other}` for `debug`"));
            }
            other => operands.push(other.to_string()),
        }
    }
    if binary.is_some() && !operands.is_empty() {
        return usage_error("`debug --binary` takes no source file");
    }
    if binary.is_none() && operands.len() != 1 {
        return usage_error("`debug` needs a source file, or `--binary <path>`");
    }
    let backend = match debug::detect_backend() {
        Ok(backend) => backend,
        Err(message) => {
            eprintln!("error: {message}");
            return exit_code(EXIT_USAGE);
        }
    };

    let scratch = temp_run_dir();
    if binary.is_none() {
        if let Err(error) = std::fs::create_dir_all(&scratch) {
            eprintln!("error: cannot create the debug scratch directory: {error}");
            return exit_code(EXIT_INTERNAL);
        }
    }
    let target = match &binary {
        Some(path) => path.clone(),
        None => scratch.join(format!("debug{}", std::env::consts::EXE_SUFFIX)),
    };
    if binary.is_none() {
        let source = match read_source_with(&PathBuf::from(&operands[0]), EXIT_USAGE) {
            Ok(source) => source,
            Err(code) => {
                let _ = std::fs::remove_dir_all(&scratch);
                return code;
            }
        };
        let toolchain = match detect() {
            Ok(toolchain) => toolchain,
            Err(message) => {
                eprintln!("error: {message}");
                let _ = std::fs::remove_dir_all(&scratch);
                return exit_code(EXIT_USAGE);
            }
        };
        let built = compile_to_binary(
            &source,
            &operands[0],
            &target,
            &toolchain,
            BuildOptions {
                debug_info: true,
                release: false,
                ir_cache: false,
                ..BuildOptions::default()
            },
        );
        if let Err(diagnostics) = built {
            let mut sources = SourceMap::new();
            sources.add_file(operands[0].clone(), source);
            for diagnostic in &diagnostics {
                eprint!(
                    "{}",
                    yashy_driver::term::render_diagnostic(&sources, diagnostic)
                );
            }
            let _ = std::fs::remove_dir_all(&scratch);
            return exit_code(EXIT_COMPILE_ERROR);
        }
    }

    // The summary providers travel with the compiler, so a user does not have
    // to find the source tree. They are written beside the script under the
    // name their module registration assumes; a write failure is reported but
    // does not stop the session -- the debugger still stops and prints task
    // state, only values fall back to addresses.
    let printers = scratch.join(debug::PRETTY_PRINTERS_FILE);
    let printers_path = match std::fs::write(&printers, debug::PRETTY_PRINTERS) {
        Ok(()) => Some(printers),
        Err(error) => {
            eprintln!("warning: cannot write the value printers: {error}");
            None
        }
    };
    let text = debug::script(
        &backend,
        &target,
        &breakpoints,
        &program_args,
        printers_path.as_deref(),
    );
    let script = scratch.join(format!("debug.{}", backend.name()));
    if let Err(error) = std::fs::write(&script, text) {
        eprintln!("error: cannot write the debugger script: {error}");
        let _ = std::fs::remove_dir_all(&scratch);
        return exit_code(EXIT_INTERNAL);
    }
    eprintln!(
        "debugger: {} ({})",
        backend.name(),
        backend.executable().display()
    );
    eprintln!("script: {}", script.display());
    let result = debug::launch(&backend, &script);
    let code = match result {
        Ok(status) => exit_code(exit_status_code(status)),
        Err(error) => {
            eprintln!("error: cannot start {}: {error}", backend.name());
            exit_code(EXIT_INTERNAL)
        }
    };
    let _ = std::fs::remove_dir_all(&scratch);
    code
}

fn repl_cmd(args: &[String]) -> ExitCode {
    use std::io::{BufRead as _, Write as _};

    if !args.is_empty() {
        return usage_error("`repl` takes no arguments; use `:quit` to leave");
    }
    let mut repl = yashy_driver::repl::Repl::new();
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    println!(
        "Yashy REPL. Declarations are kept; expressions are compiled and run.\n{}",
        yashy_driver::repl::Repl::help()
    );
    loop {
        print!("yashy> ");
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        match input.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(error) => {
                eprintln!("error: cannot read a REPL line: {error}");
                return exit_code(EXIT_INTERNAL);
            }
        }
        let kind = match repl.classify(&line) {
            Ok(kind) => kind,
            Err(message) => {
                eprintln!("error: {message}");
                continue;
            }
        };
        use yashy_driver::repl::Line;
        match kind {
            Line::Empty => continue,
            Line::Exit => break,
            Line::Help => println!("{}", yashy_driver::repl::Repl::help()),
            Line::Clear => {
                repl.clear();
                println!("cleared");
            }
            Line::Declarations => {
                if repl.declarations().is_empty() {
                    println!("(no declarations yet)");
                } else {
                    print!("{}", repl.declarations());
                }
            }
            Line::Declaration => {
                let program = repl.program_for_declaration(&line);
                let mut sources = SourceMap::new();
                match check_and_verify(
                    &mut sources,
                    Path::new("<repl>"),
                    program,
                    false,
                    &OPTIONS,
                    0,
                ) {
                    Ok(_) => {
                        repl.accept(&line);
                        println!("ok");
                    }
                    Err(diagnostics) => {
                        for diagnostic in &diagnostics {
                            eprint!(
                                "{}",
                                yashy_driver::term::render_diagnostic(&sources, diagnostic)
                            );
                        }
                    }
                }
            }
            Line::Expression => {
                let program = repl.program_for(&line);
                let mut sources = SourceMap::new();
                match check_and_verify(
                    &mut sources,
                    Path::new("<repl>"),
                    program.clone(),
                    false,
                    &OPTIONS,
                    0,
                ) {
                    Ok((hir, _)) => {
                        let ty = yashy_driver::repl::Repl::result_type(&hir)
                            .unwrap_or_else(|| "unknown".to_string());
                        repl_expression(&program, &ty);
                    }
                    Err(diagnostics) => {
                        for diagnostic in &diagnostics {
                            eprint!(
                                "{}",
                                yashy_driver::term::render_diagnostic(&sources, diagnostic)
                            );
                        }
                    }
                }
            }
        }
    }
    exit_code(EXIT_OK)
}

/// Compile and run one generated REPL program, then remove its scratch files.
fn repl_expression(program: &str, ty: &str) {
    let toolchain = match detect() {
        Ok(toolchain) => toolchain,
        Err(message) => {
            eprintln!("error: {message}");
            return;
        }
    };
    let dir = temp_run_dir();
    if let Err(error) = std::fs::create_dir_all(&dir) {
        eprintln!("error: cannot create the REPL scratch directory: {error}");
        return;
    }
    let out = dir.join(format!("repl{}", std::env::consts::EXE_SUFFIX));
    let options = BuildOptions {
        ir_cache: false,
        ..BuildOptions::default()
    };
    let built = compile_to_binary(program, "<repl>", &out, &toolchain, options);
    if let Err(diagnostics) = built {
        let mut sources = SourceMap::new();
        sources.add_file("<repl>".to_string(), program.to_string());
        for diagnostic in &diagnostics {
            eprint!(
                "{}",
                yashy_driver::term::render_diagnostic(&sources, diagnostic)
            );
        }
    } else {
        match status_with_retry_env(&out, &[], &[]) {
            Ok(_) => println!("type: {ty}"),
            Err(error) => eprintln!("error: cannot run the REPL expression: {error}"),
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

fn tasks(args: &[String]) -> ExitCode {
    run_as(args, RunMode::Tasks)
}

fn run_as(args: &[String], mode: RunMode) -> ExitCode {
    let mut build_args: Vec<String> = Vec::new();
    let mut program_args: Vec<String> = Vec::new();
    let mut input: Option<PathBuf> = None;
    let mut after_separator = false;
    let mut debug = false;
    let mut deterministic = false;
    let mut sandbox = false;
    let mut allow_read: Vec<PathBuf> = Vec::new();
    let mut allow_write: Vec<PathBuf> = Vec::new();
    let mut sandbox_time: u64 = 5000;
    let mut sandbox_memory: u64 = 256;
    // The output cap: how many bytes the program may print before the
    // sandbox stops it. The playground sets it low, because a program in a
    // browser tab has no room for a gigabyte of output.
    let mut sandbox_output: u64 = 1024 * 1024;
    // The triple the program is built for. A program built for another
    // platform cannot be run here, so it is refused rather than started.
    let mut target: Option<&'static str> = None;

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
            "--debug" => {
                // `tasks` already runs debug; the flag is never forwarded
                // to the build, only to the child's environment.
                debug = true;
            }
            "--deterministic" => deterministic = true,
            "--sandbox" => sandbox = true,
            "--allow-read" | "--allow-write" => {
                let Some(value) = iter.next() else {
                    return usage_error(&format!("`{argument}` needs a value"));
                };
                if argument == "--allow-read" {
                    allow_read.push(PathBuf::from(value));
                } else {
                    allow_write.push(PathBuf::from(value));
                }
            }
            "--time" | "--memory" | "--output-cap" => {
                let Some(value) = iter.next() else {
                    return usage_error(&format!("`{argument}` needs a value"));
                };
                match value.parse::<u64>() {
                    Ok(parsed) => {
                        if argument == "--time" {
                            sandbox_time = parsed;
                        } else if argument == "--memory" {
                            sandbox_memory = parsed;
                        } else {
                            sandbox_output = parsed;
                        }
                    }
                    Err(_) => return usage_error(&format!("`{argument}` needs a whole number")),
                }
            }
            "--release" | "--fast-math" | "--native" | "--timings" | "--no-verify" | "--no-lto"
            | "--offline" | "--stdlib-source" | "--reproducible" => {
                if mode == RunMode::Tasks && argument == "--release" {
                    return usage_error("`tasks` needs a debug build; `--release` is refused");
                }
                build_args.push(argument.clone())
            }
            "--pgo" => {
                return usage_error(
                    "`--pgo` trains a build and would run the program twice; \
                     use `yashyc build --pgo` and run the binary",
                )
            }
            "--cpu" | "--jobs" => {
                let Some(value) = iter.next() else {
                    return usage_error(&format!("`{argument}` needs a value"));
                };
                build_args.push(argument.clone());
                build_args.push(value.clone());
            }
            "--target" => {
                let Some(value) = iter.next() else {
                    return usage_error("`--target` needs a triple");
                };
                match target_flag(value) {
                    Ok(triple) => target = Some(triple),
                    Err(code) => return code,
                }
                build_args.push(argument.clone());
                build_args.push(value.clone());
            }
            other
                if other.starts_with("--cpu=")
                    || other.starts_with("--jobs=")
                    || other.starts_with("--target=") =>
            {
                build_args.push(argument.clone())
            }
            other if other.starts_with("--no-opt=") => build_args.push(argument.clone()),
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
    if debug {
        // Debug builds carry a line table so a native debugger can stop on a
        // source line and `inspect --addr` can map an address back to one.
        build_args.push("--debug-info".to_string());
    }

    // A binary built for another platform cannot run on this machine: the
    // loader would refuse it with a message about the wrong architecture,
    // which says nothing about the flag that caused it. `build --target` is
    // the way to make a program for another platform.
    if let Some(triple) = target {
        if triple != host_target_triple() {
            return usage_error(&format!(
                "`run` builds a program for this machine; `--target {triple}` builds one for \
                 another platform, which cannot be run here"
            ));
        }
    }

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

    let code = build(CommandKind::Run, &build_arguments);
    if code != exit_code(EXIT_OK) {
        let _ = fs::remove_dir_all(&dir);
        return code;
    }

    // The debug environment is what turns the runtime's panic path into a
    // task-aware one; it travels only to the child, never to the build.
    let mut child_env: Vec<(String, String)> = Vec::new();
    if debug || mode == RunMode::Tasks {
        child_env.push(("YASHY_DEBUG".to_string(), "1".to_string()));
    }
    if mode == RunMode::Tasks {
        child_env.push(("YASHY_TASKS_DUMP".to_string(), "1".to_string()));
    }
    if deterministic {
        child_env.push(("YASHY_DETERMINISTIC".to_string(), "1".to_string()));
        child_env.push((
            "YASHY_DETERMINISTIC_SEED".to_string(),
            "1700000000000".to_string(),
        ));
    }
    if debug {
        // The debugger workflow is a generated command file, not a live
        // debugging session: LLDB is launched by the user with
        // `lldb -s .yashy/debug.lldb run`. The file sets a breakpoint on
        // the entry function and dumps the async task table on stop; the
        // same table is available without a debugger through
        // `YASHY_DEBUG=1` above.
        let lldb_dir = PathBuf::from(".yashy");
        if fs::create_dir_all(&lldb_dir).is_ok() {
            let file = lldb_dir.join("debug.lldb");
            let _ = fs::write(&file, yashy_driver::profile::lldb_commands("main"));
            eprintln!("debugger commands: {}", file.display());
        }
    }
    if sandbox {
        let mut capabilities = yashy_driver::sandbox::CapabilitySet {
            time_ms: sandbox_time,
            memory_mb: sandbox_memory,
            output_bytes: sandbox_output,
            ..Default::default()
        };
        for root in &allow_read {
            match yashy_driver::sandbox::canonical_root(root) {
                Ok(canonical) => capabilities.fs_read.push(canonical),
                Err(message) => {
                    eprintln!("error: {message}");
                    return exit_code(EXIT_USAGE);
                }
            }
        }
        for root in &allow_write {
            match yashy_driver::sandbox::canonical_root(root) {
                Ok(canonical) => capabilities.fs_write.push(canonical),
                Err(message) => {
                    eprintln!("error: {message}");
                    return exit_code(EXIT_USAGE);
                }
            }
        }
        for (key, value) in yashy_driver::sandbox::sandbox_env(&capabilities) {
            child_env.push((key, value));
        }
        // The resource side: a Job Object on Windows, rlimits on Linux.
        #[cfg(target_os = "windows")]
        let job = yashy_driver::sandbox::windows_job(&capabilities);
        #[cfg(not(target_os = "windows"))]
        let job = Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "no job object",
        ));
        let child_env_refs: Vec<(&str, &str)> = child_env
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
            .collect();
        let status =
            status_with_sandbox(&output, &program_args, &child_env_refs, &capabilities, job);
        let _ = fs::remove_dir_all(&dir);
        return match status {
            Ok(status) => std::process::exit(exit_status_code(status)),
            Err(error) => {
                eprintln!("error: cannot run {}: {error}", output.display());
                exit_code(EXIT_COMPILE_ERROR)
            }
        };
    }
    let child_env_refs: Vec<(&str, &str)> = child_env
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect();
    let status = status_with_retry_env(&output, &program_args, &child_env_refs);
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
    if let Some(code) = status.code() {
        return code;
    }
    // A child killed by a signal has no exit code. A wrapper that answers 1
    // hides *which* signal and whether the child crashed; the honest answer is
    // to die from the same signal, so the shell sees 128+N and a runner can
    // tell a SIGSEGV from a SIGTERM. Resetting the handler first is required:
    // Rust may have installed one for the signal, and `raise` would then run
    // that handler instead of terminating.
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(sig) = status.signal() {
            extern "C" {
                fn signal(signal: i32, handler: usize) -> usize;
                fn raise(signal: i32) -> i32;
            }
            // SIG_DFL is 0 on every Unix the compiler targets.
            unsafe {
                signal(sig, 0);
                raise(sig);
            }
        }
    }
    EXIT_COMPILE_ERROR
}

/// Run the child with the sandbox's resource limits applied: a Job Object
/// on Windows, rlimits before exec on Linux.
fn status_with_sandbox(
    exe: &Path,
    args: &[String],
    envs: &[(&str, &str)],
    capabilities: &yashy_driver::sandbox::CapabilitySet,
    job: Result<usize, std::io::Error>,
) -> std::io::Result<ExitStatus> {
    for attempt in 0..LAUNCH_ATTEMPTS {
        let candidate = launch_candidate(exe, attempt)?;
        let mut command = Command::new(&candidate);
        command.args(args);
        for (key, value) in envs {
            command.env(key, value);
        }
        // rlimits go on the command before spawn; the Job Object is created
        // before spawn and assigned after.
        yashy_driver::sandbox::apply_limits(&mut command, capabilities)?;
        match command.spawn() {
            Ok(mut child) => {
                #[cfg(target_os = "windows")]
                {
                    if let Ok(job) = job {
                        yashy_driver::sandbox::assign_child(job, &child)?;
                    }
                }
                // The time and output limits are the driver's: poll the
                // child, kill it past `--time`, and refuse output past the
                // cap. This is portable, which the OS primitives alone are
                // not (a child that writes fast needs the cap somewhere).
                let started = std::time::Instant::now();
                loop {
                    match child.try_wait()? {
                        Some(status) => return Ok(status),
                        None => {
                            if started.elapsed().as_millis() > capabilities.time_ms as u128 {
                                let _ = child.kill();
                                let status = child.wait()?;
                                eprintln!(
                                    "error[E1901]: sandbox limit breached: time ({} ms)",
                                    capabilities.time_ms
                                );
                                return Ok(status);
                            }
                            std::thread::sleep(std::time::Duration::from_millis(10));
                        }
                    }
                }
            }
            Err(error) if blocked_by_policy(&error) => continue,
            Err(error) => return Err(error),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        "the operating system refused to start the compiled program",
    ))
}

/// Start a freshly linked binary, stepping around an Application Control /
/// Device Guard policy that refuses new content. The workaround is the same
/// one the integration tests use: a copy with extra bytes past the image is a
/// new question, and the loader never reads those bytes. The tail has to differ
/// in content, not only in length: a copy padded with zeros is still refused.
///
/// `envs` is the child's extra environment: the debug modes (`run --debug`,
/// `tasks`) use it to hand the runtime its task table triggers; an empty
/// `envs` is an ordinary run.
fn status_with_retry_env(
    exe: &Path,
    args: &[String],
    envs: &[(&str, &str)],
) -> std::io::Result<ExitStatus> {
    status_with_retry_impl(exe, args, envs, false)
}

/// As `status_with_retry_env`, but the child's stdout goes to the null device.
///
/// `profile` writes its report on stdout; a measured program that prints would
/// otherwise sit in front of a `--json` document and make it unparseable. The
/// child's stderr stays inherited, because a fault is still the user's to see.
fn status_with_retry_quiet(
    exe: &Path,
    args: &[String],
    envs: &[(&str, &str)],
) -> std::io::Result<ExitStatus> {
    status_with_retry_impl(exe, args, envs, true)
}

fn status_with_retry_impl(
    exe: &Path,
    args: &[String],
    envs: &[(&str, &str)],
    quiet: bool,
) -> std::io::Result<ExitStatus> {
    for attempt in 0..LAUNCH_ATTEMPTS {
        let candidate = launch_candidate(exe, attempt)?;
        let mut command = Command::new(&candidate);
        command.args(args);
        if quiet {
            command.stdout(Stdio::null());
        }
        for (key, value) in envs {
            command.env(key, value);
        }
        match command.status() {
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

/// `yashyc inspect [--json] <file.ya> [symbol]`: one symbol's facts, from the
/// same index `yashyc index` renders.
fn inspect_cmd(args: &[String]) -> ExitCode {
    let mut json = true;
    let mut layout = false;
    let mut layout_all = false;
    let mut rc = false;
    let mut address: Option<u64> = None;
    let mut binary: Option<PathBuf> = None;
    let mut operands: Vec<String> = Vec::new();
    let mut iter = args.iter();
    while let Some(argument) = iter.next() {
        match argument.as_str() {
            "--json" => json = true,
            "--human" => json = false,
            "--layout" => layout = true,
            "--all" => layout_all = true,
            "--rc" => rc = true,
            "--addr" => {
                let Some(value) = iter.next() else {
                    return usage_error("`--addr` needs a hexadecimal address");
                };
                let digits = value.strip_prefix("0x").unwrap_or(value);
                match u64::from_str_radix(digits, 16) {
                    Ok(parsed) => address = Some(parsed),
                    Err(_) => return usage_error("`--addr` needs a hexadecimal address"),
                }
            }
            "--binary" => {
                let Some(value) = iter.next() else {
                    return usage_error("`--binary` needs a binary path");
                };
                binary = Some(PathBuf::from(value));
            }
            other if other.starts_with('-') => {
                return usage_error(&format!("unknown option `{other}` for `inspect`"));
            }
            other => operands.push(other.to_string()),
        }
    }
    let Some(file) = operands.first() else {
        return usage_error("`inspect` needs a source file");
    };
    let path = PathBuf::from(file);
    if rc {
        return inspect_rc_cmd(&path, json);
    }
    if layout {
        let source = match read_source_with(&path, EXIT_USAGE) {
            Ok(source) => source,
            Err(code) => return code,
        };
        let mut sources = SourceMap::new();
        let checked = check_and_verify(&mut sources, &path, source, false, &OPTIONS, 0);
        let Ok((hir, _)) = checked else {
            eprintln!("error: the source must check before a layout can be reported");
            return exit_code(EXIT_COMPILE_ERROR);
        };
        let decls = yashy_driver::layout::HirDecls::from_program(&hir);
        if !json && layout_all {
            print!("{}", yashy_driver::layout::render_human(&decls));
            return exit_code(EXIT_OK);
        }
        if layout_all {
            println!("{}", yashy_driver::layout::json_report_all(&decls).render());
            return exit_code(EXIT_OK);
        }
        let Some(type_name) = operands.get(1) else {
            return usage_error("`inspect --layout` needs a type name, or `--all`");
        };
        if yashy_driver::layout::find(type_name, &decls).is_none() {
            eprintln!("error: no layout for `{type_name}`");
            return exit_code(EXIT_USAGE);
        }
        if !json {
            let line = yashy_driver::layout::human_line(type_name, &decls)
                .expect("the type was found above");
            println!("{line}");
            return exit_code(EXIT_OK);
        }
        let report = yashy_driver::layout::json_report(type_name, &decls)
            .expect("the type was found above");
        println!("{}", report.render());
        return exit_code(EXIT_OK);
    }
    // `Path::new("main.ya").parent()` is the empty path, not `.`: indexing an
    // empty root collects no files, so a relative source name would report
    // "no symbol" for every symbol it has.
    let root = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let model = match yashy_driver::index::build_index(root) {
        Ok(model) => model,
        Err(error) => {
            eprintln!("error: {error}");
            return exit_code(EXIT_USAGE);
        }
    };
    if let Some(address) = address {
        let Some(binary) = binary else {
            return usage_error("`inspect --addr` needs `--binary <path>`");
        };
        let symbols = match yashy_driver::addr::symbols(&binary) {
            Ok(symbols) => symbols,
            Err(message) => {
                eprintln!("error: {message}");
                return exit_code(EXIT_USAGE);
            }
        };
        let Some(symbol) = yashy_driver::addr::nearest(&symbols, address) else {
            eprintln!("error: no symbol at or below 0x{address:x}");
            return exit_code(EXIT_USAGE);
        };
        let source_symbol = model.symbols.iter().find(|entry| {
            symbol.name == entry.name || symbol.name == yashy_codegen::ir::fn_symbol(&entry.name)
        });
        let Some(entry) = source_symbol else {
            eprintln!("error: symbol `{}` is not a Yashy function", symbol.name);
            return exit_code(EXIT_USAGE);
        };
        let (line, column) = position_of(&entry.file, entry.span.start);
        println!(
            "{}",
            yashy_driver::addr::json(address, symbol, &entry.file, line, column, entry.span,)
                .render()
        );
        return exit_code(EXIT_OK);
    }
    let wanted = operands.get(1).map(String::as_str);
    let selected: Vec<&yashy_driver::index::Symbol> = match wanted {
        Some(name) => {
            let found: Vec<_> = model
                .symbols
                .iter()
                .filter(|symbol| symbol.name == name)
                .collect();
            if found.is_empty() {
                eprintln!("error: no symbol named `{name}`");
                return exit_code(EXIT_USAGE);
            }
            found
        }
        None => model.symbols.iter().collect(),
    };
    let layout_decls = if json
        && selected
            .iter()
            .any(|symbol| symbol.kind == "struct" || symbol.kind == "enum")
    {
        match fs::read_to_string(&path) {
            Ok(source) => {
                let mut sources = SourceMap::new();
                match check_and_verify(&mut sources, &path, source, false, &OPTIONS, 0) {
                    Ok((hir, _)) => Some(yashy_driver::layout::HirDecls::from_program(&hir)),
                    Err(_) => None,
                }
            }
            Err(_) => None,
        }
    } else {
        None
    };
    if json {
        let symbols: Vec<Json> = selected
            .iter()
            .map(|symbol| {
                let mut object = Json::object();
                object.set("name", Json::string(&symbol.name));
                object.set("kind", Json::string(&symbol.kind));
                object.set("file", Json::string(&symbol.file));
                object.set("signature", Json::string(&symbol.signature));
                object.set("visibility", Json::string(&symbol.visibility));
                // A declared type's layout belongs to the symbol's facts, not
                // to a separate command: `inspect --json <file> Point` and
                // `inspect --layout <file> Point` answer the same question,
                // so a script should not have to learn a second verb.
                if let Some(decls) = &layout_decls {
                    if symbol.kind == "struct" || symbol.kind == "enum" {
                        if let Some(found) = yashy_driver::layout::find(&symbol.name, decls) {
                            let names = decls.field_names(&symbol.name);
                            let fields: Vec<Json> = found
                                .offsets
                                .iter()
                                .enumerate()
                                .map(|(index, offset)| {
                                    let mut field = Json::object();
                                    field.set("offset", Json::number(*offset as i64));
                                    if let Some(name) = names.get(index) {
                                        field.set("field", Json::string(name));
                                    }
                                    field
                                })
                                .collect();
                            let mut layout = Json::object();
                            layout.set("size", Json::number(found.size as i64));
                            layout.set("align", Json::number(found.align as i64));
                            layout.set("fields", Json::array(fields));
                            object.set("layout", layout);
                        }
                    }
                }
                let mut span = Json::object();
                span.set("start", Json::number(i64::from(symbol.span.start)));
                span.set("end", Json::number(i64::from(symbol.span.end)));
                object.set("span", span);
                if let Some(docs) = &symbol.docs {
                    object.set("docs", Json::string(docs));
                }
                let effects: Vec<Json> = symbol
                    .effects
                    .map(|(reads, writes, net)| {
                        vec![("reads", reads), ("writes", writes), ("net", net)]
                            .into_iter()
                            .map(|(name, value)| {
                                let mut effect = Json::object();
                                effect.set("kind", Json::string(name));
                                effect.set("present", Json::bool(value));
                                effect
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                object.set("effects", Json::array(effects));
                object.set(
                    "contracts",
                    Json::array(
                        symbol
                            .contracts
                            .iter()
                            .map(|(kind, text, span)| {
                                let mut contract = Json::object();
                                contract.set("kind", Json::string(kind));
                                contract.set("text", Json::string(text));
                                contract.set("start", Json::number(i64::from(span.start)));
                                contract.set("end", Json::number(i64::from(span.end)));
                                contract
                            })
                            .collect(),
                    ),
                );
                object
            })
            .collect();
        let mut report = Json::object();
        report.set("schema_version", Json::number(1));
        report.set("tool", Json::string("inspect"));
        report.set("symbols", Json::array(symbols));
        println!("{}", report.render());
    } else {
        for symbol in selected {
            let (line, column) = position_of(&symbol.file, symbol.span.start);
            let text = format!("{}:{}:{}: {}", symbol.file, line, column, symbol.signature);
            println!(
                "{}",
                yashy_driver::term::link(
                    &yashy_driver::term::file_url(&symbol.file, line, column),
                    &text
                )
            );
        }
    }
    exit_code(EXIT_OK)
}

/// The 1-based line and column of a byte offset in a file.
///
/// `inspect` prints a position so an editor can jump to it and a terminal can
/// link it. A file that cannot be read falls back to `1:1` rather than
/// refusing the whole report: the signature is still a fact.
fn position_of(path: &str, offset: u32) -> (u32, u32) {
    let Ok(source) = std::fs::read_to_string(path) else {
        return (1, 1);
    };
    let offset = (offset as usize).min(source.len());
    let mut line = 1u32;
    let mut column = 1u32;
    for byte in source.as_bytes().iter().take(offset) {
        if *byte == b'\n' {
            line += 1;
            column = 1;
        } else {
            column += 1;
        }
    }
    (line, column)
}

/// `yashyc tokens <file.ya> [--json]` or `yashyc tokens --grammar <dir>`.
///
/// The first form is the lexer's own view of a file. The second regenerates
/// every committed editor grammar from the same token classes the lexer uses.
fn tokens(args: &[String]) -> ExitCode {
    let mut json = false;
    let mut bpe = false;
    let mut grammar: Option<PathBuf> = None;
    let mut operands: Vec<String> = Vec::new();
    let mut iter = args.iter();
    while let Some(argument) = iter.next() {
        match argument.as_str() {
            "--json" => json = true,
            "--bpe" => bpe = true,
            "--grammar" => {
                let Some(value) = iter.next() else {
                    return usage_error("`--grammar` needs a directory");
                };
                grammar = Some(PathBuf::from(value));
            }
            other if other.starts_with('-') => {
                return usage_error(&format!("unknown option `{other}` for `tokens`"));
            }
            other => operands.push(other.to_string()),
        }
    }
    if let Some(dir) = grammar {
        if !operands.is_empty() || bpe {
            return usage_error("`tokens --grammar` takes no source file");
        }
        let written = match yashy_driver::grammar::write_all(&dir) {
            Ok(files) => files,
            Err(error) => {
                eprintln!("error: cannot write the editor grammars: {error}");
                return exit_code(EXIT_USAGE);
            }
        };
        if json {
            let paths: Vec<Json> = written
                .iter()
                .map(|path| Json::string(path.to_string_lossy().replace('\\', "/")))
                .collect();
            let mut report = Json::object();
            report.set("schema_version", Json::number(1));
            report.set("tool", Json::string("tokens"));
            report.set("written", Json::array(paths));
            println!("{}", report.render());
        } else {
            println!(
                "wrote {} editor files under {}",
                written.len(),
                dir.display()
            );
        }
        return exit_code(EXIT_OK);
    }
    if bpe {
        if operands.is_empty() {
            return usage_error("`tokens --bpe` needs a file or directory");
        }
        let mut paths: Vec<PathBuf> = Vec::new();
        for operand in &operands {
            let path = PathBuf::from(operand);
            if path.is_dir() {
                let mut found = Vec::new();
                yashy_driver::index::collect_files_from(&path, &mut found);
                found.sort();
                paths.extend(found);
            } else {
                paths.push(path);
            }
        }
        return match yashy_driver::tokens::bpe_json(&paths) {
            Ok(report) => {
                println!("{}", report.render());
                exit_code(EXIT_OK)
            }
            Err(message) => {
                eprintln!("error: {message}");
                exit_code(EXIT_USAGE)
            }
        };
    }
    let Some(file) = operands.first() else {
        return usage_error("`tokens` needs a source file, or `--grammar <dir>`");
    };
    if operands.len() > 1 {
        return usage_error("`tokens` takes one source file");
    }
    let source = match read_source_with(&PathBuf::from(file), EXIT_USAGE) {
        Ok(source) => source,
        Err(code) => return code,
    };
    match yashy_driver::tokens::dump_json(&source, file) {
        Ok(report) => {
            println!("{}", report.render());
            exit_code(EXIT_OK)
        }
        Err(message) => {
            eprintln!("error: {message}");
            exit_code(EXIT_COMPILE_ERROR)
        }
    }
}

/// `yashyc cache verify <file.ya>`: recompute the v2 key for a source and say
/// whether the store holds a matching entry. A stale entry is an error: the
/// whole point of key v2 is that every input is named, so a mismatch is a bug.
fn cache_cmd(args: &[String]) -> ExitCode {
    if args.first().map(String::as_str) != Some("verify") {
        return usage_error("usage: yashyc cache verify <file.ya>");
    }
    let Some(path) = args.get(1) else {
        return usage_error("`cache verify` needs a file");
    };
    let source = match fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) => {
            eprintln!("error: cannot read {path}: {error}");
            return exit_code(EXIT_USAGE);
        }
    };
    let mut sources = SourceMap::new();
    let name = path.clone();
    sources.add_file(name.clone(), source);
    let key = yashy_driver::ir_cache_key(&sources, &name, &BuildOptions::default());
    let store = yashy_driver::ir_cache_store();
    let hit = store.as_ref().and_then(|store| store.get(&key)).is_some();
    println!("{key} {}", if hit { "hit" } else { "miss" });
    exit_code(EXIT_OK)
}

/// `yashyc watch [path]`: re-check the tree whenever a source file changes.
fn record_cmd(args: &[String]) -> ExitCode {
    use yashy_driver::record;
    let mut source: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    let mut iter = args.iter();
    while let Some(argument) = iter.next() {
        match argument.as_str() {
            "--out" => {
                let Some(value) = iter.next() else {
                    return usage_error("`--out` needs a trace path");
                };
                output = Some(PathBuf::from(value));
            }
            other if other.starts_with('-') => {
                return usage_error(&format!("unknown option `{other}` for `record`"))
            }
            other => {
                if source.is_some() {
                    return usage_error(&format!("unexpected extra argument `{other}`"));
                }
                source = Some(PathBuf::from(other));
            }
        }
    }
    let Some(source) = source else {
        return usage_error("`record` needs a source file");
    };
    let text = match std::fs::read_to_string(&source) {
        Ok(text) => text,
        Err(error) => {
            eprintln!("error: cannot read {}: {error}", source.display());
            return exit_code(EXIT_USAGE);
        }
    };
    let output = output.unwrap_or_else(|| PathBuf::from("trace.yat"));
    let dir = std::env::temp_dir().join(format!("yashy-record-{}", std::process::id()));
    if let Err(error) = std::fs::create_dir_all(&dir) {
        eprintln!("error: cannot create {}: {error}", dir.display());
        return exit_code(EXIT_INTERNAL);
    }
    let binary = dir.join(format!("program{}", std::env::consts::EXE_SUFFIX));
    let events_path = dir.join("events.txt");
    let compiler = match std::env::current_exe() {
        Ok(path) => path,
        Err(_) => return exit_code(EXIT_INTERNAL),
    };
    let built = Command::new(&compiler)
        .arg("build")
        .arg(&source)
        .arg("-o")
        .arg(&binary)
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    if !built {
        return exit_code(EXIT_COMPILE_ERROR);
    }
    let run = Command::new(&binary)
        .env("YASHY_DETERMINISTIC", "1")
        .env("YASHY_DETERMINISTIC_SEED", "1700000000000")
        .env("YASHY_TRACE_FILE", &events_path)
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    if !run {
        return exit_code(EXIT_INTERNAL);
    }
    let events = match std::fs::read_to_string(&events_path) {
        Ok(text) => text.lines().map(record::normalise).collect::<Vec<_>>(),
        Err(error) => {
            eprintln!("error: cannot read {}: {error}", events_path.display());
            return exit_code(EXIT_INTERNAL);
        }
    };
    match record::write(&record::program_hash(&text), &events, &output) {
        Ok(()) => {
            println!("recorded {}", output.display());
            exit_code(EXIT_OK)
        }
        Err(error) => {
            eprintln!("error: cannot write trace: {error}");
            exit_code(EXIT_USAGE)
        }
    }
}

fn replay_cmd(args: &[String]) -> ExitCode {
    use yashy_driver::record;
    let mut trace_path: Option<PathBuf> = None;
    let mut source: Option<PathBuf> = None;
    let mut json = false;
    let mut iter = args.iter();
    while let Some(argument) = iter.next() {
        match argument.as_str() {
            "--json" => json = true,
            "--source" => {
                let Some(value) = iter.next() else {
                    return usage_error("`--source` needs a file path");
                };
                source = Some(PathBuf::from(value));
            }
            other if other.starts_with('-') => {
                return usage_error(&format!("unknown option `{other}` for `replay`"))
            }
            other => {
                if trace_path.is_some() {
                    return usage_error(&format!("unexpected extra argument `{other}`"));
                }
                trace_path = Some(PathBuf::from(other));
            }
        }
    }
    let Some(path) = trace_path else {
        return usage_error("`replay` needs a trace file");
    };
    let trace = match record::read(&path) {
        Ok(trace) => trace,
        Err(error) => {
            eprintln!("error: {error}");
            return exit_code(EXIT_USAGE);
        }
    };
    if let Some(source_path) = source {
        let text = match std::fs::read_to_string(&source_path) {
            Ok(text) => text,
            Err(error) => {
                eprintln!("error: cannot read {}: {error}", source_path.display());
                return exit_code(EXIT_USAGE);
            }
        };
        if let Err(error) = record::validate(&trace, &text) {
            eprintln!("error: {error}");
            return exit_code(EXIT_USAGE);
        }
    }
    if json {
        println!(
            "{{\"schema_version\":1,\"tool\":\"replay\",\"program_hash\":\"{}\",\"target\":\"{}\",\"events\":{}}}",
            trace.program_hash,
            trace.target,
            trace.events.len()
        );
    } else {
        println!("{}", record::summary(&trace));
    }
    exit_code(EXIT_OK)
}

fn openapi_cmd(args: &[String]) -> ExitCode {
    let mut source: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    let mut previous: Option<PathBuf> = None;
    let mut title = "Yashy API".to_string();
    let mut iter = args.iter();
    while let Some(argument) = iter.next() {
        match argument.as_str() {
            "--out" => {
                let Some(value) = iter.next() else {
                    return usage_error("`--out` needs a path");
                };
                output = Some(PathBuf::from(value));
            }
            "--check" => {
                let Some(value) = iter.next() else {
                    return usage_error("`--check` needs a previous document");
                };
                previous = Some(PathBuf::from(value));
            }
            "--title" => {
                let Some(value) = iter.next() else {
                    return usage_error("`--title` needs a name");
                };
                title = value.clone();
            }
            other if other.starts_with('-') => {
                return usage_error(&format!("unknown option `{other}` for `openapi`"))
            }
            other => {
                if source.is_some() {
                    return usage_error(&format!("unexpected extra argument `{other}`"));
                }
                source = Some(PathBuf::from(other));
            }
        }
    }
    let Some(source) = source else {
        return usage_error("`openapi` needs a source file");
    };
    let text = match std::fs::read_to_string(&source) {
        Ok(text) => text,
        Err(error) => {
            eprintln!("error: cannot read {}: {error}", source.display());
            return exit_code(EXIT_USAGE);
        }
    };
    let found = yashy_driver::openapi::routes(&text);
    let missing = yashy_driver::openapi::incomplete(&found);
    if !missing.is_empty() {
        for line in &missing {
            eprintln!("error: {line}");
        }
        return exit_code(EXIT_USAGE);
    }
    let document = yashy_driver::openapi::document(&title, &found);
    if let Some(old_path) = previous {
        let old_text = match std::fs::read_to_string(&old_path) {
            Ok(text) => text,
            Err(error) => {
                eprintln!("error: cannot read {}: {error}", old_path.display());
                return exit_code(EXIT_USAGE);
            }
        };
        let Ok(old_value) = yashy_common::json_rpc::parse(&old_text) else {
            return usage_error("the previous document is not valid JSON");
        };
        let result = yashy_driver::openapi::check(&old_value, &document);
        println!("{}", result.as_str());
        return if result == yashy_driver::openapi::Compatibility::Breaking {
            ExitCode::from(1)
        } else {
            exit_code(EXIT_OK)
        };
    }
    let rendered = yashy_driver::openapi::render(&document);
    match output {
        Some(path) => {
            if let Err(error) = std::fs::write(&path, format!("{rendered}\n")) {
                eprintln!("error: cannot write {}: {error}", path.display());
                return exit_code(EXIT_USAGE);
            }
        }
        None => println!("{rendered}"),
    }
    exit_code(EXIT_OK)
}

fn client_cmd(args: &[String]) -> ExitCode {
    let mut from: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    let mut iter = args.iter();
    while let Some(argument) = iter.next() {
        match argument.as_str() {
            "--from" => {
                let Some(value) = iter.next() else {
                    return usage_error("`--from` needs an OpenAPI document");
                };
                from = Some(PathBuf::from(value));
            }
            "--out" => {
                let Some(value) = iter.next() else {
                    return usage_error("`--out` needs a path");
                };
                output = Some(PathBuf::from(value));
            }
            other if other.starts_with('-') => {
                return usage_error(&format!("unknown option `{other}` for `client`"))
            }
            other => {
                if from.is_some() {
                    return usage_error(&format!("unexpected extra argument `{other}`"));
                }
                from = Some(PathBuf::from(other));
            }
        }
    }
    let Some(from) = from else {
        return usage_error("`client` needs `--from <document.json>`");
    };
    let text = match std::fs::read_to_string(&from) {
        Ok(text) => text,
        Err(error) => {
            eprintln!("error: cannot read {}: {error}", from.display());
            return exit_code(EXIT_USAGE);
        }
    };
    let Ok(document) = yashy_common::json_rpc::parse(&text) else {
        return usage_error("the OpenAPI document is not valid JSON");
    };
    let source = yashy_driver::openapi::client_source(&document);
    match output {
        Some(path) => {
            if let Err(error) = std::fs::write(&path, source) {
                eprintln!("error: cannot write {}: {error}", path.display());
                return exit_code(EXIT_USAGE);
            }
        }
        None => print!("{source}"),
    }
    exit_code(EXIT_OK)
}

fn dev_cmd(args: &[String]) -> ExitCode {
    let mut path: Option<PathBuf> = None;
    let mut json = false;
    let mut interval_ms = 50u64;
    let mut iter = args.iter();
    while let Some(argument) = iter.next() {
        match argument.as_str() {
            "--json" => json = true,
            "--interval" => {
                let Some(value) = iter.next() else {
                    return usage_error("`--interval` needs a value");
                };
                match value.parse::<u64>() {
                    Ok(value) => interval_ms = value,
                    Err(_) => return usage_error("`--interval` needs a whole number"),
                }
            }
            other if other.starts_with('-') => {
                return usage_error(&format!("unknown option `{other}` for `dev`"))
            }
            other => {
                if path.is_some() {
                    return usage_error(&format!("unexpected extra argument `{other}`"));
                }
                path = Some(PathBuf::from(other));
            }
        }
    }
    let path = path.unwrap_or_else(|| PathBuf::from("."));
    let (root, only) = if path.is_file() {
        (path.parent().unwrap_or(Path::new(".")).to_path_buf(), Some(path))
    } else {
        (path, None)
    };
    match yashy_driver::dev::dev(yashy_driver::dev::DevOptions {
        root,
        only,
        json,
        interval_ms,
    }) {
        Ok(()) => exit_code(EXIT_OK),
        Err(error) => {
            eprintln!("error: cannot run the dev loop: {error}");
            exit_code(EXIT_USAGE)
        }
    }
}

fn watch_cmd(args: &[String]) -> ExitCode {
    let mut path: Option<PathBuf> = None;
    let mut json = false;
    let mut run = false;
    let mut test = false;
    let mut interval_ms = 50u64;
    let mut iter = args.iter();
    while let Some(argument) = iter.next() {
        match argument.as_str() {
            "--json" => json = true,
            "--run" => run = true,
            "--test" => test = true,
            "--interval" => {
                let Some(value) = iter.next() else {
                    return usage_error("`--interval` needs a value");
                };
                match value.parse::<u64>() {
                    Ok(value) => interval_ms = value,
                    Err(_) => return usage_error("`--interval` needs a whole number"),
                }
            }
            other if other.starts_with('-') => {
                return usage_error(&format!("unknown option `{other}` for `watch`"))
            }
            other => {
                if path.is_some() {
                    return usage_error(&format!("unexpected extra argument `{other}`"));
                }
                path = Some(PathBuf::from(other));
            }
        }
    }
    let path = path.unwrap_or_else(|| PathBuf::from("."));
    let (root, only) = if path.is_file() {
        (
            path.parent().unwrap_or(Path::new(".")).to_path_buf(),
            Some(path),
        )
    } else {
        (path, None)
    };
    let options = yashy_driver::watch::WatchOptions {
        root,
        only,
        json,
        run,
        test,
        interval_ms,
    };
    match yashy_driver::watch::watch(options) {
        Ok(()) => exit_code(EXIT_OK),
        Err(error) => {
            eprintln!("error: cannot watch: {error}");
            exit_code(EXIT_USAGE)
        }
    }
}

/// `yashyc serve [path]`: keep a warm compiler daemon until stdin closes.
///
/// The daemon is optional: every other command works one-shot. It keeps the
/// source map, query cache and toolchain warm and answers the same seven
/// compiler verbs over a local JSON-RPC socket.
fn serve(args: &[String]) -> ExitCode {
    if args.len() > 1 {
        return usage_error("`serve` takes at most one path");
    }
    let root = args
        .first()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let handle = match yashy_driver::daemon::DaemonHandle::start(&root) {
        Ok(handle) => handle,
        Err(error) => {
            eprintln!("error: cannot start the daemon: {error}");
            return exit_code(EXIT_INTERNAL);
        }
    };
    println!("yashy daemon listening on {}", handle.addr());
    use std::io::BufRead as _;
    let mut line = String::new();
    let _ = std::io::stdin().lock().read_line(&mut line);
    handle.shutdown();
    exit_code(EXIT_OK)
}

/// `yashyc test [path] [--filter <substring>] [--list]`.
///
/// R1 tests are ordinary programs. A file under `tests/` is one test; the
/// process exit code is the verdict. This keeps the language free of
/// attributes and macros while still giving a project a real test command.
fn test(args: &[String]) -> ExitCode {
    let mut root: Option<PathBuf> = None;
    let mut workspace = false;
    let mut package: Option<String> = None;
    let mut filter: Option<String> = None;
    let mut list = false;
    let mut json = false;
    let mut timings_flag = false;
    let mut jobs: usize = 0;
    let mut target: Option<&'static str> = None;
    let mut affected: Option<PathBuf> = None;
    let mut allowed_external: Vec<String> = Vec::new();

    let mut iter = args.iter();
    let mut sandbox = false;
    while let Some(argument) = iter.next() {
        match argument.as_str() {
            "--list" => list = true,
            "--workspace" => workspace = true,
            "-p" | "--package" => {
                let Some(value) = iter.next() else {
                    return usage_error("`-p` needs a member name");
                };
                package = Some(value.clone());
            }
            "--sandbox" => sandbox = true,
            "--allow-path" => {
                let Some(value) = iter.next() else {
                    return usage_error("`--allow-path` needs a path");
                };
                allowed_external.push(value.clone());
            }
            "--affected" => {
                let Some(value) = iter.next() else {
                    return usage_error("`--affected` needs a changed-files list");
                };
                affected = Some(PathBuf::from(value));
            }
            "--json" => json = true,
            "--timings" => timings_flag = true,
            "--target" => {
                let Some(value) = iter.next() else {
                    return usage_error("`--target` needs a triple");
                };
                match target_flag(value) {
                    Ok(triple) => target = Some(triple),
                    Err(code) => return code,
                }
            }
            other if other.starts_with("--target=") => {
                match target_flag(&other["--target=".len()..]) {
                    Ok(triple) => target = Some(triple),
                    Err(code) => return code,
                }
            }
            "--jobs" => {
                let Some(value) = iter.next() else {
                    return usage_error("`--jobs` needs a value");
                };
                match value.parse::<usize>() {
                    Ok(parsed) => {
                        jobs = parsed;
                    }
                    Err(_) => return usage_error("`--jobs` needs a whole number"),
                }
            }
            "--filter" => {
                let Some(value) = iter.next() else {
                    return usage_error("`--filter` needs a substring");
                };
                filter = Some(value.clone());
            }
            other if other.starts_with("--jobs=") => {
                match other["--jobs=".len()..].parse::<usize>() {
                    Ok(parsed) => {
                        jobs = parsed;
                    }
                    Err(_) => return usage_error("`--jobs` needs a whole number"),
                }
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
    if workspace || package.is_some() {
        let root_dir = root.clone().unwrap_or_else(|| PathBuf::from("."));
        let found = match yashy_driver::workspace::load(&root_dir) {
            Ok(found) => found,
            Err(message) => {
                eprintln!("error: {message}");
                return exit_code(EXIT_USAGE);
            }
        };
        let found = match found {
            Some(found) => found,
            None => {
                eprintln!("error: {} is not a workspace root", root_dir.display());
                return exit_code(EXIT_USAGE);
            }
        };
        let members = match found.select(package.as_deref(), workspace, &allowed_external) {
            Ok(members) => members,
            Err(message) => {
                eprintln!("error: {message}");
                return exit_code(EXIT_USAGE);
            }
        };
        // Forward every other flag to each member as an ordinary single-root
        // test run; removing the workspace flags keeps the recursion finite.
        let mut rest: Vec<String> = Vec::new();
        let mut index = 0;
        while index < args.len() {
            match args[index].as_str() {
                "--workspace" => {}
                "-p" | "--package" => index += 1,
                "--allow-path" if index + 1 < args.len() => {
                    rest.push("--allow-path".to_string());
                    rest.push(args[index + 1].clone());
                    index += 1;
                }
                other if other.starts_with('-') => rest.push(other.to_string()),
                _ => {}
            }
            index += 1;
        }
        let mut code = exit_code(EXIT_OK);
        for member in members {
            let mut member_args = vec![member.to_string_lossy().into_owned()];
            member_args.extend(rest.clone());
            let result = test(&member_args);
            if result != exit_code(EXIT_OK) {
                code = result;
            }
        }
        return code;
    }

    // `test --sandbox`: every test program runs with the default-deny set
    // (read/write limited to the test root itself).
    let root_for_sandbox = root.clone().unwrap_or_else(|| PathBuf::from("tests"));
    let sandbox_env: Vec<(String, String)> = if sandbox {
        let canonical = root_for_sandbox
            .canonicalize()
            .unwrap_or(root_for_sandbox.clone());
        let mut capabilities = yashy_driver::sandbox::CapabilitySet::default();
        capabilities.fs_read.push(canonical.clone());
        capabilities.fs_write.push(canonical);
        yashy_driver::sandbox::sandbox_env(&capabilities)
    } else {
        Vec::new()
    };
    let sandbox_env_refs: Vec<(&str, &str)> = sandbox_env
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect();

    let explicit_root = root.is_some();
    let root = root.unwrap_or_else(|| PathBuf::from("tests"));
    // A test is a program that is built *and run*, so a test built for another
    // platform could not be run here.
    if let Some(triple) = target {
        if triple != host_target_triple() {
            return usage_error(&format!(
                "`test` runs the programs it builds; `--target {triple}` builds them for \
                 another platform, which cannot be run here"
            ));
        }
    }
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

    let affected_selection = if let Some(list_path) = &affected {
        let text = match fs::read_to_string(list_path) {
            Ok(text) => text,
            Err(error) => {
                eprintln!("error: cannot read {}: {error}", list_path.display());
                return exit_code(EXIT_USAGE);
            }
        };
        let changed: Vec<PathBuf> = text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .map(PathBuf::from)
            .collect();
        let selection = yashy_driver::affected::select(&root, &tests, &changed);
        tests.retain(|path| {
            let name = path
                .strip_prefix(&root)
                .unwrap_or(path)
                .to_string_lossy()
                .replace('\\', "/");
            selection.selected.contains(&name)
        });
        Some(selection)
    } else {
        None
    };

    if json {
        return test_json(
            &root,
            &tests,
            timings_flag,
            jobs,
            target.unwrap_or_else(host_target_triple),
            &sandbox_env_refs,
            affected_selection.as_ref(),
        );
    }

    if let Some(selection) = &affected_selection {
        println!(
            "selected {} of {} tests",
            selection.selected.len(),
            selection.selected.len() + selection.skipped.len()
        );
        for (name, reason) in &selection.skipped {
            println!("skip {name}: {reason}");
        }
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

    let toolchain = match detect() {
        Ok(toolchain) => toolchain,
        Err(message) => {
            eprintln!("error: {message}");
            return exit_code(EXIT_USAGE);
        }
    };
    let report = test_harness::run_suite(
        &tests,
        &toolchain,
        target.unwrap_or_else(host_target_triple),
        jobs,
        &sandbox_env_refs,
    );

    if let Some(message) = &report.internal_error {
        eprintln!("error: {message}");
        return exit_code(EXIT_INTERNAL);
    }

    for result in &report.results {
        match result.status {
            TestStatus::Passed => println!("test {} ... ok", result.file),
            TestStatus::Failed => {
                let suffix = result
                    .exit
                    .map(|code| format!(" (exit {code})"))
                    .unwrap_or_default();
                println!("test {} ... FAILED{}", result.file, suffix);
                print!("{}", result.stdout);
                eprint!("{}", result.stderr);
            }
            TestStatus::CompileError => {
                println!("test {} ... FAILED (compile error)", result.file);
                for message in &result.messages {
                    eprintln!("{message}");
                }
            }
        }
    }

    println!();
    println!(
        "test result: {}. {} passed; {} failed; {} compile errors",
        if report.failed == 0 && report.compile_errors == 0 {
            "ok"
        } else {
            "FAILED"
        },
        report.passed,
        report.failed,
        report.compile_errors,
    );
    if timings_flag {
        report_timings(&report.timings);
    }
    if report.failed == 0 && report.compile_errors == 0 {
        exit_code(EXIT_OK)
    } else {
        exit_code(EXIT_COMPILE_ERROR)
    }
}

/// `yashyc test --json`: the same test discovery as `test`, with one JSON
/// envelope instead of the human progress lines.
///
/// Compile diagnostics are collected through the same checker and code
/// generator the human path uses, so a fact in the JSON (code, severity, file,
/// span) is the fact the human renderer prints.
fn test_json(
    root: &Path,
    tests: &[PathBuf],
    timings_flag: bool,
    jobs: usize,
    target: &'static str,
    sandbox_env: &[(&str, &str)],
    selection: Option<&yashy_driver::affected::Selection>,
) -> ExitCode {
    let mut global_sources = SourceMap::new();
    let _root_file = global_sources.add_file(root.display().to_string(), String::new());

    if tests.is_empty() {
        print_test_json(&global_sources, &[], &[], &[], 0, 0, 0, None, None);
        return exit_code(EXIT_OK);
    }

    let toolchain = match detect() {
        Ok(toolchain) => toolchain,
        Err(message) => {
            let diagnostic = Diagnostic::new(
                Severity::Error,
                FileId::from_index(0),
                Span::SYNTHETIC,
                message,
            );
            print_test_json(
                &global_sources,
                &[diagnostic],
                &[],
                &[],
                0,
                0,
                0,
                None,
                None,
            );
            return exit_code(EXIT_USAGE);
        }
    };

    let report = test_harness::run_suite(tests, &toolchain, target, jobs, sandbox_env);
    if let Some(message) = &report.internal_error {
        let diagnostic = Diagnostic::new(
            Severity::Error,
            FileId::from_index(0),
            Span::SYNTHETIC,
            message.clone(),
        );
        print_test_json(
            &global_sources,
            &[diagnostic],
            &[],
            &[],
            0,
            0,
            0,
            None,
            None,
        );
        return exit_code(EXIT_INTERNAL);
    }
    let test_objects: Vec<Json> = report
        .results
        .iter()
        .map(|result| {
            let mut object = Json::object();
            object.set("name", Json::string(&result.name));
            object.set("file", Json::string(&result.file));
            object.set("status", Json::string(result.status.as_str()));
            object.set("duration_ms", Json::number(result.duration_ms));
            object.set("stdout", Json::string(&result.stdout));
            object.set("stderr", Json::string(&result.stderr));
            if let Some(exit) = result.exit {
                object.set("exit", Json::number(i64::from(exit)));
            }
            object.set("diagnostics", Json::array(result.diagnostics.clone()));
            object.set("failure", Json::string(failure_message(result)));
            object
        })
        .collect();
    let events: Vec<Json> = report
        .results
        .iter()
        .flat_map(|result| test_events(result))
        .collect();

    print_test_json(
        &global_sources,
        &[],
        &test_objects,
        &events,
        report.passed,
        report.failed,
        report.compile_errors,
        timings_flag.then_some(report.timings.as_slice()),
        selection.map(yashy_driver::affected::to_json),
    );
    if report.failed == 0 && report.compile_errors == 0 {
        exit_code(EXIT_OK)
    } else {
        exit_code(EXIT_COMPILE_ERROR)
    }
}

/// The failure message a runner reports for one test.
///
/// A run failure owns its exit status and stderr; a compile error owns its
/// diagnostics. A passing test has no message, so consumers can treat an empty
/// string as "nothing went wrong" without checking the status first.
fn failure_message(result: &yashy_driver::test_harness::TestResult) -> String {
    match result.status {
        yashy_driver::test_harness::TestStatus::Passed => String::new(),
        yashy_driver::test_harness::TestStatus::Failed => {
            let exit = result
                .exit
                .map(|code| format!("exit {code}: "))
                .unwrap_or_default();
            let detail = if result.stderr.is_empty() {
                result.stdout.trim().to_string()
            } else {
                result.stderr.trim().to_string()
            };
            format!("{exit}{detail}")
        }
        yashy_driver::test_harness::TestStatus::CompileError => result
            .diagnostics
            .iter()
            .map(|diagnostic| match diagnostic {
                Json::Object(fields) => fields
                    .iter()
                    .find(|(key, _)| key == "message")
                    .and_then(|(_, value)| match value {
                        Json::String(text) => Some(text.clone()),
                        _ => None,
                    })
                    .unwrap_or_default(),
                _ => String::new(),
            })
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

/// The start and end events for one test, in that order.
fn test_events(result: &yashy_driver::test_harness::TestResult) -> [Json; 2] {
    let mut start = Json::object();
    start.set("event", Json::string("start"));
    start.set("name", Json::string(&result.name));
    start.set("file", Json::string(&result.file));

    let mut end = Json::object();
    end.set("event", Json::string("end"));
    end.set("name", Json::string(&result.name));
    end.set("file", Json::string(&result.file));
    end.set("status", Json::string(result.status.as_str()));
    end.set("duration_ms", Json::number(result.duration_ms));
    end.set("stdout", Json::string(&result.stdout));
    end.set("failure", Json::string(failure_message(result)));
    if let Some(exit) = result.exit {
        end.set("exit", Json::number(i64::from(exit)));
    }
    [start, end]
}

fn print_test_json(
    global_sources: &SourceMap,
    global: &[Diagnostic],
    tests: &[Json],
    events: &[Json],
    passed: usize,
    failed: usize,
    compile_errors: usize,
    timings: Option<&[TimingRow]>,
    selection: Option<Json>,
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
    report.set("events", Json::array(events.to_vec()));
    if let Some(selection) = selection {
        report.set("selection", selection);
    }
    report.set("summary", summary);
    set_timings(&mut report, timings);
    println!("{}", report.render());
}

/// Add the `timings` array to an envelope when the flag was passed.
///
/// Omitted rather than empty when it was not: a report that never asked for
/// timings should stay byte-identical to the one this compiler printed before
/// the flag existed, and an empty array would be a new field for every caller.
fn set_timings(report: &mut Json, timings: Option<&[TimingRow]>) {
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
                    match timing {
                        TimingRow::Phase { phase, ms } => {
                            entry.set("phase", Json::string(*phase));
                            entry.set("ms", Json::number(*ms as i64));
                        }
                        TimingRow::Query(stats) => {
                            entry.set("phase", Json::string("query"));
                            entry.set("hits", Json::number(stats.hits as i64));
                            entry.set("misses", Json::number(stats.misses as i64));
                            entry.set(
                                "recomputed",
                                Json::array(
                                    stats
                                        .recomputed
                                        .iter()
                                        .map(|name| Json::string(name))
                                        .collect(),
                                ),
                            );
                        }
                        TimingRow::FrontendThreads { threads } => {
                            entry.set("phase", Json::string("frontend_threads"));
                            entry.set("threads", Json::number(*threads as i64));
                        }
                        TimingRow::Promotion { stack, arena } => {
                            entry.set("phase", Json::string("promotion"));
                            entry.set("promoted_stack", Json::number(*stack as i64));
                            entry.set("promoted_arena", Json::number(*arena as i64));
                        }
                        TimingRow::Lto { mode } => {
                            entry.set("phase", Json::string("lto"));
                            entry.set("mode", Json::string(*mode));
                        }
                        TimingRow::Pgo { stage } => {
                            entry.set("phase", Json::string("pgo"));
                            entry.set("stage", Json::string(*stage));
                        }
                        TimingRow::Instantiations { count } => {
                            entry.set("phase", Json::string("monomorphise"));
                            entry.set("instantiations", Json::number(*count as i64));
                        }
                        TimingRow::Tasks { count } => {
                            entry.set("phase", Json::string("tasks"));
                            entry.set("tasks", Json::number(*count as i64));
                        }
                        TimingRow::IrCache { hit } => {
                            entry.set("phase", Json::string("ir_cache"));
                            entry.set("hit", Json::bool(*hit));
                        }
                        TimingRow::StdlibModules { count } => {
                            entry.set("phase", Json::string("stdlib_modules"));
                            entry.set("modules", Json::number(*count as i64));
                        }
                        TimingRow::ModuleObjects { rebuilt, cached } => {
                            entry.set("phase", Json::string("module_objects"));
                            entry.set("rebuilt", Json::number(*rebuilt as i64));
                            entry.set("cached", Json::number(*cached as i64));
                        }
                    }
                    entry
                })
                .collect(),
        ),
    );
}

/// Print one `timing:` line per phase and one `query_*` line, to stderr.
///
/// stderr is deliberate: stdout belongs to the program under `run` and to the
/// machine-readable envelope under `--json`, and a timing line in either would
/// corrupt it. An empty slice prints nothing, which is how a caller says the
/// flag was not passed.
fn report_timings(timings: &[TimingRow]) {
    for timing in timings {
        match timing {
            TimingRow::Phase { phase, ms } => eprintln!("timing: {phase} {ms} ms"),
            TimingRow::Query(stats) => eprintln!(
                "query_hits={} query_misses={} recomputed={}",
                stats.hits,
                stats.misses,
                stats.recomputed.join(",")
            ),
            TimingRow::FrontendThreads { threads } => eprintln!("frontend_threads={threads}"),
            TimingRow::Promotion { stack, arena } => {
                eprintln!("promoted_stack={stack} promoted_arena={arena}")
            }
            TimingRow::Lto { mode } => eprintln!("lto={mode}"),
            TimingRow::Pgo { stage } => eprintln!("pgo={stage}"),
            TimingRow::Instantiations { count } => eprintln!("instantiations={count}"),
            TimingRow::Tasks { count } => eprintln!("tasks={count}"),
            TimingRow::IrCache { hit } => {
                eprintln!("ir_cache={}", if *hit { "hit" } else { "miss" })
            }
            TimingRow::StdlibModules { count } => eprintln!("stdlib_modules={count}"),
            TimingRow::ModuleObjects { rebuilt, cached } => {
                eprintln!("timing: modules {rebuilt} rebuilt, {cached} cached")
            }
        }
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

/// `yashyc explain <code> [--json]`.
///
/// The human form prints the embedded catalog page verbatim; the JSON form
/// carries the same facts so an agent can read the title, summary, and page
/// without scraping Markdown. A code that is not in the catalog is a usage
/// error: the command cannot explain something that does not exist, and a
/// near miss names the code the reader probably meant.
fn explain(args: &[String]) -> ExitCode {
    let mut json = false;
    let mut all = false;
    let mut catalog = false;
    let mut code: Option<&str> = None;

    for argument in args {
        match argument.as_str() {
            "--json" => json = true,
            "--all" => all = true,
            "--catalog" => catalog = true,
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

    if catalog && !all {
        return usage_error("`--catalog` needs `--all`");
    }
    if all && catalog {
        let path = Path::new("docs/errors.md");
        let document = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) => {
                eprintln!("error: {}: {error}", path.display());
                return exit_code(EXIT_USAGE);
            }
        };
        let rows: Vec<String> = codes::CODES
            .iter()
            .map(|entry| {
                format!(
                    "| [{}](errors/{}.md) | {} |",
                    entry.id, entry.id, entry.title
                )
            })
            .collect();
        let mut lines: Vec<String> = document.lines().map(str::to_string).collect();
        let Some(start) = lines
            .iter()
            .position(|line| line.trim() == "| Code | Meaning |")
        else {
            return usage_error("docs/errors.md has no `| Code | Meaning |` table");
        };
        let mut end = start + 2;
        while end < lines.len() && lines[end].starts_with('|') {
            end += 1;
        }
        lines.splice(start + 2..end, rows);
        println!("{}", lines.join("\n"));
        return exit_code(EXIT_OK);
    }
    if all {
        if code.is_some() {
            return usage_error("`--all` cannot be combined with a code");
        }
        let mut report = Json::object();
        report.set("schema_version", Json::number(1));
        report.set("tool", Json::string("explain"));
        report.set("yashy_version", Json::string(env!("CARGO_PKG_VERSION")));
        report.set("total", Json::number(codes::CODES.len() as i64));
        let entries: Vec<Json> = codes::CODES
            .iter()
            .map(|entry| {
                let mut value = Json::object();
                value.set("id", Json::string(entry.id));
                value.set("title", Json::string(entry.title));
                value.set("summary", Json::string(entry.summary));
                value.set("rule", Json::string(codes::rule_for(entry.id)));
                value.set("docs_url", Json::string(codes::docs_url(entry.id)));
                value
            })
            .collect();
        report.set("codes", Json::array(entries));
        println!("{}", report.render());
        return exit_code(EXIT_OK);
    }

    let Some(code) = code else {
        return usage_error("`explain` needs an error code, for example `yashyc explain E0100`");
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
        yashy_driver::term::page(entry.page);
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
    report.set("rule", Json::string(codes::rule_for(code.id)));
    report.set("page", Json::string(code.page));
    report.set("docs_url", Json::string(codes::docs_url(code.id)));
    println!("{}", report.render());
}

/// `yashyc task <name>`: run a named sequence of compiler verbs.
fn task_cmd(args: &[String]) -> ExitCode {
    let tasks = match yashy_driver::task::load(Path::new(".")) {
        Ok(tasks) => tasks,
        Err(error) => {
            eprintln!("error: {error}");
            return exit_code(EXIT_USAGE);
        }
    };
    let names: Vec<&str> = tasks.iter().map(|task| task.name.as_str()).collect();
    if args.first().map(String::as_str) == Some("--list") {
        for name in &names {
            println!("{name}");
        }
        return exit_code(EXIT_OK);
    }
    let Some(name) = args.first() else {
        return usage_error("`task` needs a name (or `--list`)");
    };
    let Some(task) = tasks.iter().find(|task| task.name == *name) else {
        eprintln!(
            "error: no task named `{name}`; available: {}",
            names.join(", ")
        );
        return exit_code(EXIT_USAGE);
    };
    let known: Vec<&str> = yashy_driver::cli_spec::COMMANDS
        .iter()
        .map(|command| command.name)
        .collect();
    if let Err(error) = yashy_driver::task::validate(&[task.clone()], &known) {
        eprintln!("error: {error}");
        return exit_code(EXIT_USAGE);
    }
    for step in &task.steps {
        let code = dispatch(step);
        if code != exit_code(EXIT_OK) {
            return code;
        }
    }
    exit_code(EXIT_OK)
}

/// `yashyc new <name> [--template <name>] [--list]`.
fn new_cmd(args: &[String]) -> ExitCode {
    let mut template = "bin".to_string();
    let mut name: Option<String> = None;
    let mut list = false;
    let mut iter = args.iter();
    while let Some(argument) = iter.next() {
        match argument.as_str() {
            "--template" => {
                let Some(value) = iter.next() else {
                    return usage_error("`--template` needs a name");
                };
                template = value.clone();
            }
            "--list" => list = true,
            other if other.starts_with('-') => {
                return usage_error(&format!("unknown option `{other}` for `new`"));
            }
            other => {
                if name.is_some() {
                    return usage_error(&format!("unexpected extra argument `{other}`"));
                }
                name = Some(other.to_string());
            }
        }
    }
    if list {
        for item in yashy_driver::scaffold::TEMPLATES {
            println!("{item}");
        }
        return exit_code(EXIT_OK);
    }
    let Some(name) = name else {
        return usage_error("`new` needs a project name");
    };
    match yashy_driver::scaffold::create(Path::new("."), &name, &template) {
        Ok(created) => {
            println!(
                "created {} ({template}, edition 2027)",
                created.root.display()
            );
            exit_code(EXIT_OK)
        }
        Err(error) => {
            eprintln!("error: {error}");
            exit_code(EXIT_USAGE)
        }
    }
}

/// `yashyc init [path]`.
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
            "fn main() {\n    print(\"Hello, Yashy!\");\n}\n".to_string(),
        ),
        (
            "tests/smoke.ya",
            "fn main() {\n    assert(1 + 1 == 2);\n}\n".to_string(),
        ),
        (
            "README.md",
            format!(
                "# {name}\n\nA Yashy project.\n\n## Run\n\n```text\nyashyc run main.ya\n```\n\n## Test\n\n```text\nyashyc test\n```\n\n## Build\n\n```text\nyashyc build main.ya -o {name} --release\n```\n\nSee `AGENTS.md` for the project layout and the rules an editor or agent should follow.\n"
            ),
        ),
        (
            "AGENTS.md",
            format!(
                "# {name}\n\nThis is a Yashy project. `main.ya` is the entry point; the programs under `tests/` are the test suite.\n\n## Commands\n\n```text\nyashyc check main.ya\nyashyc run main.ya\nyashyc test\nyashyc build main.ya -o {name} --release\n```\n\n## Layout\n\n- `main.ya` -- the program's entry point.\n- `tests/` -- one test program per file. A test is an ordinary Yashy program with a `main`; exit 0 is a pass.\n- `yashy.toml` -- package metadata.\n- `AGENTS.md` -- this guide.\n\n## Rules\n\n- `let` is a mutable binding; `const` is immutable.\n- A module's items are private unless they say `pub`. Import a file by its stem, and keep the file beside the importer.\n- A file under `tests/` is its own entry file. It cannot import a module from the project root by walking up; keep a test self-contained or test the built binary.\n- Errors are values: `Option[T]` for a value that may be missing, `Result[T, E]` for an operation that may fail. There are no exceptions.\n- Owned values are released explicitly: `defer` for a scope, `.free()` for arrays, maps, strings built at run time, channels, and mutexes. The checker refuses a use after `move`.\n- Run `yashyc test` before calling the work done. A compile error is not a test failure; fix the diagnostic first.\n\n## Docs\n\n- [Quickstart](https://github.com/Yash-K-Jagani/Yashy/blob/main/docs/quickstart.md)\n- [Language tutorial](https://github.com/Yash-K-Jagani/Yashy/blob/main/docs/tutorial/00-intro.md)\n- [Errors](https://github.com/Yash-K-Jagani/Yashy/blob/main/docs/errors.md)\n- [First CLI](https://github.com/Yash-K-Jagani/Yashy/blob/main/docs/tutorial/12-first-cli.md)\n"
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
    println!("  yashyc run main.ya");
    println!("  yashyc test");
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
    lsp: Option<PathBuf>,
    path: PathReport,
    next: Option<String>,
    /// The triple this report is about: the host, or the one `--target` named.
    target: &'static str,
    /// Whether the C toolchain can link for `target`, when that is not the
    /// host. `None` when no cross target was asked about.
    cross: Option<CrossReport>,
}

/// Whether the target's C toolchain is installed.
struct CrossReport {
    ok: bool,
    error: Option<String>,
    /// The triple this report is about.
    target: &'static str,
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

/// `yashyc doctor [--json]`: the things a fresh install can get wrong.
fn doctor(args: &[String]) -> ExitCode {
    let mut json = false;
    let mut target: Option<&'static str> = None;
    let mut iter = args.iter();
    while let Some(argument) = iter.next() {
        match argument.as_str() {
            "--json" => json = true,
            "--target" => {
                let Some(value) = iter.next() else {
                    return usage_error("`--target` needs a triple");
                };
                match target_flag(value) {
                    Ok(triple) => target = Some(triple),
                    Err(code) => return code,
                }
            }
            other if other.starts_with("--target=") => {
                match target_flag(&other["--target=".len()..]) {
                    Ok(triple) => target = Some(triple),
                    Err(code) => return code,
                }
            }
            other => return usage_error(&format!("unexpected extra argument `{other}`")),
        }
    }

    let report = collect_doctor_report(target);
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

fn collect_doctor_report(target: Option<&'static str>) -> DoctorReport {
    let clang = check_clang();
    let stdlib = check_stdlib();
    let path = check_path();
    let lsp = {
        let candidate = lsp_sibling();
        if candidate.is_file() {
            Some(candidate)
        } else {
            None
        }
    };
    // A cross target needs its own C toolchain; the host's `clang` may well
    // have the backend but not the target's CRT and linker. The probe asks by
    // building, because that is the only question that has a truthful answer.
    let target = target.unwrap_or_else(host_target_triple);
    let cross = if target == host_target_triple() {
        None
    } else {
        Some(match detect() {
            Ok(toolchain) => match yashy_driver::target::probe_cross(&toolchain, target) {
                Ok(()) => CrossReport {
                    ok: true,
                    error: None,
                    target,
                },
                Err(message) => CrossReport {
                    ok: false,
                    error: Some(message),
                    target,
                },
            },
            Err(message) => CrossReport {
                ok: false,
                error: Some(message),
                target,
            },
        })
    };
    let ok = clang.ok && stdlib.ok && cross.as_ref().map(|c| c.ok).unwrap_or(true);
    let next = if ok {
        None
    } else {
        Some(doctor_next(&clang, &stdlib, cross.as_ref()))
    };

    DoctorReport {
        ok,
        clang,
        stdlib,
        lsp,
        path,
        next,
        target,
        cross,
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

fn doctor_next(clang: &ClangReport, stdlib: &StdlibReport, cross: Option<&CrossReport>) -> String {
    let mut parts: Vec<String> = Vec::new();
    if !clang.ok {
        parts.push(yashy_driver::toolchain::install_hint());
    }
    if !stdlib.ok {
        parts.push(
            "Set YASHY_STDLIB to the directory containing io.ya, or reinstall Yashy.".to_string(),
        );
    }
    if let Some(cross) = cross {
        if !cross.ok {
            parts.push(format!(
                "Install the C toolchain for `{}`; a cross build links with it.",
                cross.target
            ));
        }
    }

    let mut next = parts.join(" ");
    if !next.is_empty() {
        next.push_str(" Then run `yashyc doctor`.");
    }
    next
}

fn print_doctor_human(report: &DoctorReport) {
    println!("yashyc {}", env!("CARGO_PKG_VERSION"));
    println!("target: {}", report.target);

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

    // The cross toolchain line only exists when a target was asked for, so
    // the default doctor report stays the one it always was.
    match &report.cross {
        Some(cross) if cross.ok => println!("cross toolchain: ok for {}", report.target),
        Some(cross) => {
            eprintln!("cross toolchain: missing for {}", report.target);
            if let Some(error) = &cross.error {
                eprintln!("  {error}");
            }
        }
        None => {}
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
    writeln!(out, "  \"target\": {},", json_string(report.target))
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

    match &report.lsp {
        Some(path) => {
            out.push_str("  \"lsp\": {\n");
            writeln!(out, "    \"found\": true,").expect("writing to a String cannot fail");
            writeln!(
                out,
                "    \"path\": {}",
                json_string(&path.to_string_lossy())
            )
            .expect("writing to a String cannot fail");
            out.push_str("  },\n");
        }
        None => {
            out.push_str("  \"lsp\": {\n");
            writeln!(out, "    \"found\": false,").expect("writing to a String cannot fail");
            writeln!(out, "    \"path\": null").expect("writing to a String cannot fail");
            out.push_str("  },\n");
        }
    }

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

    // The cross toolchain is only reported when a target was asked for, so a
    // reader can tell "not checked" from "checked and missing".
    match &report.cross {
        Some(cross) => {
            out.push_str("  \"cross\": {\n");
            writeln!(out, "    \"ok\": {},", cross.ok).expect("writing to a String cannot fail");
            writeln!(
                out,
                "    \"error\": {}",
                json_opt_string(cross.error.as_deref())
            )
            .expect("writing to a String cannot fail");
            out.push_str("  },\n");
        }
        None => out.push_str("  \"cross\": null,\n"),
    }

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
    timings: Option<&[TimingRow]>,
    assumptions: Option<&[Assumption]>,
    certificates: Option<&[Json]>,
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
    if let Some(certificates) = certificates {
        report.set("certificates", Json::array(certificates.to_vec()));
    }
    println!("{}", report.render());
}

/// One discharged obligation as a machine-readable certificate.
///
/// The rule is the one whose diagnostic would have fired had the fact not been
/// proved: a bounds or overflow fact belongs to `RUNTIME-2`, a shift fact to
/// `TYPE-6`. A certificate without a rule is an orphan, so this mapping is a
/// closed match rather than a guess from a string.
fn certificate_json(sources: &SourceMap, file: FileId, obligation: &ProofObligation) -> Json {
    use yashy_verify::ObligationKind;
    let rule = match obligation.kind {
        ObligationKind::Bounds | ObligationKind::Overflow => {
            yashy_common::codes::rule_for(yashy_common::codes::OVERFLOW)
        }
        ObligationKind::ShiftInRange => {
            yashy_common::codes::rule_for(yashy_common::codes::SHIFT_OUT_OF_RANGE)
        }
    };
    let mut object = Json::object();
    object.set("rule", Json::string(rule));
    object.set("obligation", Json::string(format!("{:?}", obligation.kind)));
    object.set(
        "fact",
        Json::string(if obligation.established {
            "established"
        } else {
            "runtime_check_required"
        }),
    );
    object.set("span", span_json(sources, file, obligation.span));
    object
}
fn assumption_json(sources: &SourceMap, assumption: &Assumption) -> Json {
    let mut object = Json::object();
    object.set("kind", Json::string(assumption.kind.as_str()));
    object.set("name", Json::string(assumption.name.as_str()));
    object.set("reason", Json::string(assumption.reason));
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
            "  {kind:<kind_width$}  {name:<name_width$}  {file}:{line}:{column}  {reason}",
            kind = assumption.kind.as_str(),
            name = assumption.name,
            file = sources.path(assumption.file),
            line = location.line,
            column = location.col,
            reason = assumption.reason,
        );
    }
}

/// `yashyc lint <file.ya> [--json] [--list] [--group <g>]`.
fn lint_cmd(args: &[String]) -> ExitCode {
    use yashy_driver::lint::{self, LintGroup, GROUPS, LINTS};
    let mut json = false;
    let mut list = false;
    let mut docs = false;
    let mut group: Option<LintGroup> = None;
    let mut deny: Vec<String> = Vec::new();
    let mut operands: Vec<String> = Vec::new();
    let mut iter = args.iter();
    while let Some(argument) = iter.next() {
        match argument.as_str() {
            "--json" => json = true,
            "--list" => list = true,
            "--docs" => docs = true,
            "--group" => {
                let Some(value) = iter.next() else {
                    return usage_error("`--group` needs a group name");
                };
                let Some(parsed) = LintGroup::parse(value) else {
                    let names: Vec<&str> = GROUPS.iter().map(|group| group.as_str()).collect();
                    return usage_error(&format!(
                        "unknown lint group `{value}`; expected one of {}",
                        names.join(", ")
                    ));
                };
                group = Some(parsed);
            }
            "--deny" => {
                let Some(value) = iter.next() else {
                    return usage_error("`--deny` needs a lint code");
                };
                if !LINTS.iter().any(|lint| lint.code == value) {
                    return usage_error(&format!("unknown lint code `{value}` for `--deny`"));
                }
                deny.push(value.clone());
            }
            other if other.starts_with('-') => {
                return usage_error(&format!("unknown option `{other}` for `lint`"));
            }
            other => operands.push(other.to_string()),
        }
    }
    if docs {
        print!("{}", lint::render_docs());
        return exit_code(EXIT_OK);
    }
    if list {
        if json {
            let entries: Vec<Json> = LINTS
                .iter()
                .map(|lint| {
                    let mut object = Json::object();
                    object.set("code", Json::string(lint.code));
                    object.set("group", Json::string(lint.group.as_str()));
                    object.set("message", Json::string(lint.message));
                    object.set("hint", Json::string(lint.hint));
                    object
                })
                .collect();
            let mut report = Json::object();
            report.set("lints", Json::array(entries));
            println!("{}", report.render());
        } else {
            for lint in LINTS {
                println!(
                    "{}  {:<12} {}",
                    lint.code,
                    lint.group.as_str(),
                    lint.message
                );
            }
        }
        return exit_code(EXIT_OK);
    }
    let Some(file) = operands.first() else {
        return usage_error("`lint` needs a source file (or `--list`)");
    };
    let source = match read_source_with(&PathBuf::from(file), EXIT_USAGE) {
        Ok(source) => source,
        Err(code) => return code,
    };
    let diagnostics = lint::check_filtered(&source, FileId::from_index(0), group);
    let denied = diagnostics.iter().any(|diagnostic| {
        diagnostic
            .code
            .is_some_and(|code| deny.iter().any(|deny| deny == code))
    });
    if json {
        let items: Vec<Json> = diagnostics
            .iter()
            .map(|diagnostic| diagnostic_json(&SourceMap::new(), diagnostic))
            .collect();
        let mut report = Json::object();
        report.set("lints", Json::array(items));
        report.set("denied", Json::bool(denied));
        println!("{}", report.render());
    } else {
        let mut map = SourceMap::new();
        map.add_file(file.clone(), source);
        for diagnostic in &diagnostics {
            eprint!(
                "{}",
                yashy_driver::term::render_diagnostic(&map, diagnostic)
            );
        }
        if diagnostics.is_empty() {
            println!("no lints in {file}");
        }
    }
    if denied {
        return exit_code(EXIT_COMPILE_ERROR);
    }
    exit_code(EXIT_OK)
}

/// `yashyc fix <file.ya> [--dry-run]`: apply machine-applicable edits, but only
/// to the exact bytes the diagnostic was computed from.
fn fix_cmd(args: &[String]) -> ExitCode {
    let mut dry_run = false;
    let mut input: Option<PathBuf> = None;
    for argument in args {
        match argument.as_str() {
            "--dry-run" => dry_run = true,
            other if other.starts_with('-') => {
                return usage_error(&format!("unknown option `{other}` for `fix`"));
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
        return usage_error("`fix` needs a source file");
    };
    let source = match read_source_with(&input, EXIT_USAGE) {
        Ok(source) => source,
        Err(code) => return code,
    };
    let hash = yashy_driver::fix::source_hash(&source);
    let mut sources = SourceMap::new();
    let outcome = load_and_check_exports_timed(
        &mut sources,
        &input,
        source.clone(),
        false,
        host_target_triple(),
        0,
        &mut Vec::new(),
    );
    let mut diagnostics = match outcome {
        Ok((hir, exports)) => {
            let mut warnings = Vec::new();
            yashy_verify::pass::warnings::check(&hir, &exports, &mut warnings);
            warnings
        }
        Err(diagnostics) => diagnostics,
    };
    yashy_driver::fix::attach_typo_suggestions(&sources, &mut diagnostics);
    let suggestions: Vec<yashy_common::Suggestion> = diagnostics
        .iter()
        .flat_map(|diagnostic| diagnostic.suggestions.clone())
        .collect();
    if suggestions.is_empty() {
        println!("nothing to fix in {}", input.display());
        return exit_code(EXIT_OK);
    }
    match yashy_driver::fix::apply_suggestions_checked(&source, &suggestions, Some(hash)) {
        Ok(fixed) => {
            if dry_run {
                println!("{fixed}");
            } else if let Err(error) = fs::write(&input, fixed) {
                eprintln!("error: cannot write {}: {error}", input.display());
                return exit_code(EXIT_INTERNAL);
            } else {
                println!(
                    "fixed {} diagnostic(s) in {}",
                    suggestions.len(),
                    input.display()
                );
            }
            exit_code(EXIT_OK)
        }
        Err(error) => {
            eprintln!("error: {error}");
            exit_code(EXIT_COMPILE_ERROR)
        }
    }
}

/// `yashyc check <file.ya> [--json]`.
fn check(args: &[String]) -> ExitCode {
    let mut json = false;
    let mut sarif = false;
    let mut timings_flag = false;
    let mut print_cfg = false;
    let mut target: Option<&'static str> = None;
    let mut deny_warnings = env::var("YASHY_DENY_WARNINGS").ok().as_deref() == Some("1");
    let mut allow_warnings = false;
    let mut jobs: usize = 0;
    let mut requested_features: Vec<String> = Vec::new();
    let mut no_default_features = false;
    let mut input: Option<PathBuf> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--json" => json = true,
            "--sarif" => sarif = true,
            "--timings" => timings_flag = true,
            "--print-cfg" => print_cfg = true,
            "--target" => {
                if i + 1 < args.len() {
                    match target_flag(&args[i + 1]) {
                        Ok(triple) => target = Some(triple),
                        Err(code) => return code,
                    }
                    i += 1;
                } else {
                    return usage_error("`--target` needs a triple");
                }
            }
            other if other.starts_with("--target=") => {
                match target_flag(&other["--target=".len()..]) {
                    Ok(triple) => target = Some(triple),
                    Err(code) => return code,
                }
            }
            "--jobs" => {
                if i + 1 < args.len() {
                    match args[i + 1].parse::<usize>() {
                        Ok(parsed) => jobs = parsed,
                        Err(_) => return usage_error("`--jobs` needs a whole number"),
                    }
                    i += 1;
                } else {
                    return usage_error("`--jobs` needs a value");
                }
            }
            "--deny" => {
                if i + 1 < args.len() && args[i + 1] == "warnings" {
                    deny_warnings = true;
                    i += 1;
                } else {
                    return usage_error(&format!("unknown option `--deny` for `check`"));
                }
            }
            "--allow" => {
                if i + 1 < args.len() && args[i + 1] == "warnings" {
                    allow_warnings = true;
                    i += 1;
                } else {
                    return usage_error(&format!("unknown option `--allow` for `check`"));
                }
            }
            "-W" => {
                if i + 1 < args.len() && args[i + 1] == "none" {
                    allow_warnings = true;
                    i += 1;
                } else {
                    return usage_error(&format!("unknown option `-W` for `check`"));
                }
            }
            "-W none" => allow_warnings = true,
            "--deny warnings" => deny_warnings = true,
            "--allow warnings" => allow_warnings = true,
            "--features" => {
                if i + 1 < args.len() {
                    requested_features.extend(args[i + 1].split(',').map(str::to_string));
                    i += 1;
                } else {
                    return usage_error("`--features` needs a comma-separated list");
                }
            }
            "--no-default-features" => no_default_features = true,
            other if other.starts_with("--features=") => {
                requested_features
                    .extend(other["--features=".len()..].split(',').map(str::to_string));
            }
            other if other.starts_with("--jobs=") => {
                match other["--jobs=".len()..].parse::<usize>() {
                    Ok(parsed) => jobs = parsed,
                    Err(_) => return usage_error("`--jobs` needs a whole number"),
                }
            }
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
        i += 1;
    }
    let target = target.unwrap_or_else(host_target_triple);
    // Features are read from the project the file belongs to, or the working
    // directory when only `--print-cfg` is asked for. They are decided here,
    // before `--print-cfg` prints and before the loader filters `cfg`, so the
    // keys a script reads are the keys the check used.
    let selected_features =
        match select_features(input.as_deref(), &requested_features, no_default_features) {
            Ok(features) => features,
            Err(code) => return code,
        };
    // `--print-cfg` answers "what is true for this target?" and stops: it is
    // the flag a build script reads and the one a confused user runs after
    // `cfg(window)` did not compile, so it needs no source file.
    if print_cfg {
        let set =
            yashy_driver::cfg::CfgSet::for_target(target, false).with_features(&selected_features);
        if json {
            let mut report = Json::object();
            report.set("schema_version", Json::number(1));
            report.set("tool", Json::string("check"));
            report.set("target", Json::string(target));
            report.set(
                "cfg",
                Json::array(
                    set.active_keys()
                        .iter()
                        .map(|key| Json::string(key.clone()))
                        .collect(),
                ),
            );
            println!("{}", report.render());
        } else {
            for key in set.active_keys() {
                println!("{key}");
            }
        }
        return exit_code(EXIT_OK);
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
    let measured: &mut Vec<TimingRow> = if timings_flag {
        &mut timings
    } else {
        &mut scratch
    };
    let outcome = load_and_check_exports_timed_with_features(
        &mut sources,
        &input,
        source,
        false,
        target,
        jobs,
        &selected_features,
        measured,
    );
    let reported = timings_flag.then_some(timings.as_slice());
    match outcome {
        Ok((hir, exports)) => {
            let mut warning_diags = Vec::new();
            yashy_verify::pass::warnings::check(&hir, &exports, &mut warning_diags);
            let manifest_path = input
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join("yashy.toml");
            if let Ok(manifest) = Manifest::load(&manifest_path) {
                warning_diags.extend(yashy_driver::capability_check::check(&hir, &manifest));
            }
            // Apply the warning policy.
            if allow_warnings {
                warning_diags.clear();
            } else if deny_warnings {
                for d in &mut warning_diags {
                    d.severity = Severity::Error;
                }
            }
            let has_denied = deny_warnings && !warning_diags.is_empty();
            yashy_driver::fix::attach_typo_suggestions(&sources, &mut warning_diags);
            if sarif {
                println!(
                    "{}",
                    yashy_driver::sarif::to_sarif(&sources, &warning_diags).render()
                );
            } else if json {
                let mut all = warning_diags;
                print_check_json("check", &sources, &all, reported, None, None);
            } else {
                for d in &warning_diags {
                    eprint!("{}", yashy_driver::term::render_diagnostic(&sources, d));
                }
                report_timings(&timings);
            }
            if has_denied {
                exit_code(EXIT_COMPILE_ERROR)
            } else {
                exit_code(EXIT_OK)
            }
        }
        Err(mut diagnostics) => {
            yashy_driver::fix::attach_typo_suggestions(&sources, &mut diagnostics);
            if sarif {
                println!(
                    "{}",
                    yashy_driver::sarif::to_sarif(&sources, &diagnostics).render()
                );
                if diagnostics.iter().any(|diagnostic| diagnostic.is_error()) {
                    exit_code(EXIT_COMPILE_ERROR)
                } else {
                    exit_code(EXIT_OK)
                }
            } else if json {
                let errors = diagnostics
                    .iter()
                    .filter(|diagnostic| diagnostic.is_error())
                    .count();
                print_check_json("check", &sources, &diagnostics, reported, None, None);
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

/// `yashyc verify <file.ya> [--json] [--inventory]`.
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
fn migrate_cmd(args: &[String]) -> ExitCode {
    let mut dry_run = false;
    let mut input: Option<PathBuf> = None;
    for argument in args {
        match argument.as_str() {
            "--dry-run" => dry_run = true,
            other if other.starts_with('-') => {
                return usage_error(&format!("unknown option `{other}` for `migrate`"));
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
        return usage_error("`migrate` needs a manifest path");
    };
    let source = match fs::read_to_string(&input) {
        Ok(source) => source,
        Err(error) => {
            eprintln!("error: cannot read {}: {error}", input.display());
            return exit_code(EXIT_USAGE);
        }
    };
    let migrated = match yashy_driver::migrate::rewrite(&source, "2027") {
        Ok(migrated) => migrated,
        Err(message) => {
            eprintln!("error: {message}");
            return exit_code(EXIT_USAGE);
        }
    };
    if dry_run {
        print!("{}", yashy_driver::migrate::diff(&source, &migrated));
        return exit_code(EXIT_OK);
    }
    if let Err(error) = fs::write(&input, migrated) {
        eprintln!("error: cannot write {}: {error}", input.display());
        return exit_code(EXIT_COMPILE_ERROR);
    }
    exit_code(EXIT_OK)
}

fn verify(args: &[String]) -> ExitCode {
    let mut json = false;
    let mut sarif = false;
    let mut timings_flag = false;
    let mut inventory_flag = false;
    let mut require_inventory = false;
    let mut certificates: Option<PathBuf> = None;
    let mut deny_warnings = env::var("YASHY_DENY_WARNINGS").ok().as_deref() == Some("1");
    let mut allow_warnings = false;
    let mut jobs: usize = 0;
    let mut requested_features: Vec<String> = Vec::new();
    let mut no_default_features = false;
    let mut input: Option<PathBuf> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--json" => json = true,
            "--sarif" => sarif = true,
            "--timings" => timings_flag = true,
            "--inventory" => inventory_flag = true,
            "--require-inventory" => require_inventory = true,
            "--certificates" => {
                if i + 1 < args.len() {
                    certificates = Some(PathBuf::from(&args[i + 1]));
                    i += 1;
                } else {
                    return usage_error("`--certificates` needs a file path");
                }
            }
            "--jobs" => {
                if i + 1 < args.len() {
                    match args[i + 1].parse::<usize>() {
                        Ok(parsed) => jobs = parsed,
                        Err(_) => return usage_error("`--jobs` needs a whole number"),
                    }
                    i += 1;
                } else {
                    return usage_error("`--jobs` needs a value");
                }
            }
            "--deny" => {
                if i + 1 < args.len() && args[i + 1] == "warnings" {
                    deny_warnings = true;
                    i += 1;
                } else {
                    return usage_error(&format!("unknown option `--deny` for `verify`"));
                }
            }
            "--allow" => {
                if i + 1 < args.len() && args[i + 1] == "warnings" {
                    allow_warnings = true;
                    i += 1;
                } else {
                    return usage_error(&format!("unknown option `--allow` for `verify`"));
                }
            }
            "-W" => {
                if i + 1 < args.len() && args[i + 1] == "none" {
                    allow_warnings = true;
                    i += 1;
                } else {
                    return usage_error(&format!("unknown option `-W` for `verify`"));
                }
            }
            "-W none" => allow_warnings = true,
            "--deny warnings" => deny_warnings = true,
            "--allow warnings" => allow_warnings = true,
            "--features" => {
                if i + 1 < args.len() {
                    requested_features.extend(args[i + 1].split(',').map(str::to_string));
                    i += 1;
                } else {
                    return usage_error("`--features` needs a comma-separated list");
                }
            }
            "--no-default-features" => no_default_features = true,
            other if other.starts_with("--features=") => {
                requested_features
                    .extend(other["--features=".len()..].split(',').map(str::to_string));
            }
            other if other.starts_with("--jobs=") => {
                match other["--jobs=".len()..].parse::<usize>() {
                    Ok(parsed) => jobs = parsed,
                    Err(_) => return usage_error("`--jobs` needs a whole number"),
                }
            }
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
        i += 1;
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
    let selected_features =
        match select_features(Some(&input), &requested_features, no_default_features) {
            Ok(features) => features,
            Err(code) => return code,
        };
    let measured: &mut Vec<TimingRow> = if timings_flag {
        &mut timings
    } else {
        &mut scratch
    };
    let outcome = check_and_verify_timed_with_features(
        &mut sources,
        &input,
        source,
        false,
        host_target_triple(),
        &OPTIONS,
        jobs,
        &selected_features,
        measured,
    );
    let reported = timings_flag.then_some(timings.as_slice());
    match outcome {
        Ok((hir, mut report)) => {
            // Apply the warning policy.
            if allow_warnings {
                report
                    .diagnostics
                    .retain(|d| d.severity != Severity::Warning);
            } else if deny_warnings {
                for d in &mut report.diagnostics {
                    if d.severity == Severity::Warning {
                        d.severity = Severity::Error;
                    }
                }
            }
            // ALIAS-5: an audit can require every trust assumption to be
            // listed. The warning is opt-in so an ordinary `verify` keeps its
            // existing output; it never changes the exit code.
            if require_inventory && !inventory_flag {
                let found = yashy_verify::inventory(&hir);
                if !found.is_empty() {
                    report.diagnostics.push(
                        Diagnostic::new(
                            Severity::Warning,
                            FileId::from_index(0),
                            Span::SYNTHETIC,
                            format!(
                                "{} assumption(s) were not reported; add `--inventory` to list them",
                                found.len()
                            ),
                        )
                        .with_code(codes::ASSUMPTION_NOT_REPORTED),
                    );
                }
            }
            // The inventory is computed from the checked HIR, so it exists
            // exactly when the program type-checked. It never changes the
            // exit code: the verifier accepted the program with these
            // assumptions before the flag existed, and it still does.
            let assumptions = inventory_flag.then(|| yashy_verify::inventory(&hir));
            // A certificate is written for every obligation the verifier
            // attempted, not only the established ones: a required runtime
            // check is evidence too, and hiding it would make the page lie
            // about what the proof discharged.
            let certificates_json: Option<Vec<Json>> = certificates.as_ref().map(|path| {
                let entries: Vec<Json> = report
                    .obligations
                    .iter()
                    .map(|obligation| certificate_json(&sources, FileId::from_index(0), obligation))
                    .collect();
                if let Err(error) = std::fs::write(path, Json::array(entries.clone()).render()) {
                    eprintln!("error: cannot write {}: {error}", path.display());
                }
                entries
            });
            if sarif {
                println!(
                    "{}",
                    yashy_driver::sarif::to_sarif(&sources, &report.diagnostics).render()
                );
            } else if json {
                print_check_json(
                    "verify",
                    &sources,
                    &report.diagnostics,
                    reported,
                    assumptions.as_deref(),
                    certificates_json.as_deref(),
                );
            } else {
                if !report.is_empty() {
                    for diagnostic in &report.diagnostics {
                        eprint!(
                            "{}",
                            yashy_driver::term::render_diagnostic(&sources, diagnostic)
                        );
                    }
                }
                if let Some(assumptions) = &assumptions {
                    print_inventory(&sources, assumptions);
                }
            }
            if !json && !sarif {
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
                print_check_json("verify", &sources, &diagnostics, reported, None, None);
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

/// `yashyc fmt [-w | --check] [--json] <files...>`.
///
/// Without a mode flag the formatted text goes to standard output, so a file
/// can be inspected before it is replaced. With `-w` a file is written in
/// place, and only when it parsed: an unformattable file is left exactly as it
/// was.
///
/// `--check` is the CI gate: it writes nothing, names every file that is not
/// already canonical, and exits 1 if there is at least one. The comparison
/// normalises CRLF first, because `core.autocrlf=true` is the default on
/// Windows and a gate that fails on a clean checkout is a gate people disable.
/// `--json` renders the same answer as a document, so CI need not parse text.
fn fmt(args: &[String]) -> ExitCode {
    let mut write = false;
    let mut check = false;
    let mut json = false;
    let mut files: Vec<PathBuf> = Vec::new();
    for argument in args {
        match argument.as_str() {
            "-w" | "--write" => write = true,
            "--check" => check = true,
            "--json" => json = true,
            other if other.starts_with('-') => {
                return usage_error(&format!("unknown flag `{other}` for `fmt`"));
            }
            other => files.push(PathBuf::from(other)),
        }
    }
    if files.is_empty() {
        return usage_error("`fmt` needs at least one source file");
    }
    if write && check {
        return usage_error("`fmt --check` writes nothing: drop `--write`");
    }
    if json && !check {
        return usage_error("`fmt --json` needs `--check`: there is nothing to list otherwise");
    }

    let mut failed = false;
    let mut changed: Vec<(String, bool)> = Vec::new();
    for path in &files {
        let source = match read_source_with(path, EXIT_COMPILE_ERROR) {
            Ok(source) => source,
            Err(_) => {
                failed = true;
                continue;
            }
        };
        match yashy_fmt::format(&source) {
            Ok(formatted) => {
                let differs = normalise_newlines(&source) != normalise_newlines(&formatted);
                if check {
                    if differs {
                        changed.push((path.display().to_string(), true));
                        if !json {
                            println!("{}", path.display());
                        }
                    }
                } else if write {
                    if differs {
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

    if json {
        let mut report = Json::object();
        report.set("schema_version", Json::number(1));
        report.set("tool", Json::string("fmt"));
        report.set("success", Json::bool(!failed && changed.is_empty()));
        report.set(
            "files",
            Json::array(
                changed
                    .iter()
                    .map(|(file, differs)| {
                        let mut entry = Json::object();
                        entry.set("file", Json::string(file));
                        entry.set("changed", Json::bool(*differs));
                        entry
                    })
                    .collect(),
            ),
        );
        println!("{}", report.render());
    }

    if failed {
        exit_code(EXIT_COMPILE_ERROR)
    } else if !changed.is_empty() {
        exit_code(EXIT_COMPILE_ERROR)
    } else {
        exit_code(EXIT_OK)
    }
}

/// Text with every CRLF pair folded to LF, so a comparison asks "is this file
/// canonical?" rather than "did this checkout use Windows line endings?".
fn normalise_newlines(text: &str) -> String {
    text.replace("\r\n", "\n")
}

/// Render each diagnostic with its source line and caret, to stderr.
///
/// `sources` is the map the diagnostics were produced against, so a diagnostic
/// from an imported module names that module and points into its text. Building
/// a fresh map here would put every caret in the entry file.
fn render(sources: &SourceMap, diagnostics: &[Diagnostic]) -> ExitCode {
    for diagnostic in diagnostics {
        eprint!(
            "{}",
            yashy_driver::term::render_diagnostic(&sources, diagnostic)
        );
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

/// `yashyc expand <file.ya> [--json]`.
///
/// Prints the fully expanded program after every macro has been replaced, and
/// stops before checking: this command is for reading what a macro produces.
fn expand(args: &[String]) -> ExitCode {
    let mut json = false;
    let mut input: Option<PathBuf> = None;
    for arg in args {
        match arg.as_str() {
            "--json" => json = true,
            other if other.starts_with('-') => {
                return usage_error(&format!("unknown option `{other}` for `expand`"))
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
        return usage_error("`expand` needs a source file");
    };
    let source = match read_source_with(&input, EXIT_USAGE) {
        Ok(source) => source,
        Err(code) => return code,
    };
    let mut sources = SourceMap::new();
    let file = sources.add_file(input.to_string_lossy(), source);
    let source_text = sources.file(file).to_string();

    let tokens = match yashy_lexer::Lexer::with_file(&source_text, file).tokenize() {
        Ok(tokens) => tokens,
        Err(diagnostics) => {
            for diagnostic in diagnostics {
                eprint!(
                    "{}",
                    yashy_driver::term::render_diagnostic(&sources, &diagnostic)
                );
            }
            return exit_code(EXIT_COMPILE_ERROR);
        }
    };
    let program = match yashy_parser::Parser::new(tokens.clone(), &sources, file).parse_program() {
        Ok(program) => program,
        Err(diagnostics) => {
            for diagnostic in diagnostics {
                eprint!(
                    "{}",
                    yashy_driver::term::render_diagnostic(&sources, &diagnostic)
                );
            }
            return exit_code(EXIT_COMPILE_ERROR);
        }
    };
    let (expanded, map) = yashy_expand::expand_tokens(&program.items, &sources, file, &tokens);
    let expanded = match expanded {
        Ok(tokens) => tokens,
        Err(diagnostic) => {
            eprint!(
                "{}",
                yashy_driver::term::render_diagnostic(&sources, &diagnostic)
            );
            return exit_code(EXIT_COMPILE_ERROR);
        }
    };

    if json {
        print_expand_json(&sources, file, &map);
        return exit_code(EXIT_OK);
    }

    let rendered = render_tokens(&expanded, &sources, file);
    match yashy_fmt::format(&rendered) {
        Ok(text) => {
            print!("{text}");
            exit_code(EXIT_OK)
        }
        Err(diagnostics) => {
            for diagnostic in diagnostics {
                eprint!(
                    "{}",
                    yashy_driver::term::render_diagnostic(&sources, &diagnostic)
                );
            }
            exit_code(EXIT_COMPILE_ERROR)
        }
    }
}

/// `yashyc index [path] [--json]`: the symbol index over a directory or a
/// single file.
fn index_cmd(args: &[String]) -> ExitCode {
    let mut json = false;
    let mut input: Option<PathBuf> = None;
    for arg in args {
        match arg.as_str() {
            "--json" => json = true,
            other if other.starts_with('-') => {
                return usage_error(&format!("unknown option `{other}` for `index`"))
            }
            other => {
                if input.is_some() {
                    return usage_error(&format!("unexpected extra argument `{other}`"));
                }
                input = Some(PathBuf::from(other));
            }
        }
    }
    let root = input.unwrap_or_else(|| PathBuf::from("."));
    match yashy_driver::index::build_index(&root) {
        Ok(model) => {
            if json {
                println!("{}", yashy_driver::index::to_json(&model).render());
            } else {
                print!("{}", yashy_driver::index::render_human(&model));
            }
            exit_code(EXIT_OK)
        }
        Err(message) => {
            eprintln!("error: {message}");
            exit_code(EXIT_USAGE)
        }
    }
}

/// `yashyc outline <file>`: the file's symbols in source order.
fn outline_cmd(args: &[String]) -> ExitCode {
    let mut input: Option<PathBuf> = None;
    for arg in args {
        match arg.as_str() {
            other if other.starts_with('-') => {
                return usage_error(&format!("unknown option `{other}` for `outline`"))
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
        return usage_error("`outline` needs a source file");
    };
    let root = input
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    match yashy_driver::outline::load(&input) {
        Ok(model) => {
            let file = input.to_string_lossy().replace('\\', "/");
            for line in yashy_driver::outline::outline_of(&model, &file) {
                println!("{line}");
            }
            exit_code(EXIT_OK)
        }
        Err(message) => {
            eprintln!("error: {message}");
            exit_code(EXIT_USAGE)
        }
    }
}

/// `yashyc graph [path] [--json|--dot]`: the module and call graph.
fn graph_cmd(args: &[String]) -> ExitCode {
    let mut json = false;
    let mut dot = false;
    let mut input: Option<PathBuf> = None;
    let mut from: Option<String> = None;
    let mut depth: Option<usize> = None;
    let mut edges: Vec<String> = Vec::new();
    let mut iter = args.iter();
    while let Some(argument) = iter.next() {
        match argument.as_str() {
            "--json" => json = true,
            "--dot" => dot = true,
            "--from" => match iter.next() {
                Some(value) => from = Some(value.clone()),
                None => return usage_error("`--from` needs a symbol name"),
            },
            "--depth" => match iter.next().and_then(|value| value.parse::<usize>().ok()) {
                Some(value) => depth = Some(value),
                None => return usage_error("`--depth` needs a whole number"),
            },
            "--edges" => match iter.next() {
                Some(value) => {
                    for kind in value.split(',') {
                        match kind.trim() {
                            "import" | "imports" => edges.push("import".to_string()),
                            "call" | "calls" => edges.push("call".to_string()),
                            "" => {}
                            other => {
                                return usage_error(&format!(
                                    "unknown edge kind `{other}`; the graph models import and call"
                                ))
                            }
                        }
                    }
                }
                None => return usage_error("`--edges` needs a comma-separated list"),
            },
            other if other.starts_with('-') => {
                return usage_error(&format!("unknown option `{other}` for `graph`"))
            }
            other => {
                if input.is_some() {
                    return usage_error(&format!("unexpected extra argument `{other}`"));
                }
                input = Some(PathBuf::from(other));
            }
        }
    }
    if from.is_some() && dot {
        return usage_error("`--from` is a JSON query; it cannot be combined with `--dot`");
    }
    if from.is_none() && !edges.is_empty() {
        return usage_error("`--edges` filters a `--from` query; it does nothing on the full graph");
    }
    let root = input.unwrap_or_else(|| PathBuf::from("."));
    match yashy_driver::outline::load(&root) {
        Ok(model) => {
            if let Some(symbol) = from {
                let kinds: Vec<&str> = edges.iter().map(String::as_str).collect();
                let result = yashy_driver::outline::graph_scope_json(
                    &model,
                    &symbol,
                    depth.unwrap_or(1),
                    &kinds,
                );
                println!("{}", result.render());
            } else if dot {
                print!("{}", yashy_driver::outline::graph_dot(&model));
            } else if json {
                println!("{}", yashy_driver::outline::graph_json(&model).render());
            } else {
                print!("{}", yashy_driver::outline::graph_dot(&model));
            }
            exit_code(EXIT_OK)
        }
        Err(message) => {
            eprintln!("error: {message}");
            exit_code(EXIT_USAGE)
        }
    }
}

/// `yashyc search <query> [path]`: ranked symbol search, or
/// `yashyc search <query> --registry <root>`: package registry search.
fn search_cmd(args: &[String]) -> ExitCode {
    if args.iter().any(|argument| argument == "--registry") {
        let mut query: Option<String> = None;
        let mut registry: Option<String> = None;
        let mut iter = args.iter();
        while let Some(argument) = iter.next() {
            match argument.as_str() {
                "--registry" => match iter.next() {
                    Some(value) => registry = Some(value.clone()),
                    None => return usage_error("`--registry` needs a directory"),
                },
                other if other.starts_with('-') => {
                    return usage_error(&format!("unknown option `{other}` for `search`"))
                }
                other => {
                    if query.is_some() {
                        return usage_error(&format!("unexpected extra argument `{other}`"));
                    }
                    query = Some(other.to_string());
                }
            }
        }
        let Some(registry) = registry else {
            return usage_error("`search` needs `--registry <dir>`");
        };
        let root = match yashy_driver::registry::registry_path(&registry) {
            Ok(path) => path,
            Err(error) => {
                eprintln!("error: {error}");
                return exit_code(EXIT_COMPILE_ERROR);
            }
        };
        return match yashy_driver::registry::search(&root, query.as_deref()) {
            Ok(entries) => {
                for entry in entries {
                    println!("{} {} {}", entry.name, entry.version, entry.hash);
                }
                exit_code(EXIT_OK)
            }
            Err(error) => {
                eprintln!("error: {error}");
                exit_code(EXIT_COMPILE_ERROR)
            }
        };
    }
    let mut query: Option<String> = None;
    let mut input: Option<PathBuf> = None;
    for arg in args {
        match arg.as_str() {
            other if other.starts_with('-') => {
                return usage_error(&format!("unknown option `{other}` for `search`"))
            }
            other => {
                if query.is_none() {
                    query = Some(other.to_string());
                } else if input.is_none() {
                    input = Some(PathBuf::from(other));
                } else {
                    return usage_error(&format!("unexpected extra argument `{other}`"));
                }
            }
        }
    }
    let Some(query) = query else {
        return usage_error("`search` needs a query");
    };
    let root = input.unwrap_or_else(|| PathBuf::from("."));
    match yashy_driver::outline::load(&root) {
        Ok(model) => {
            for line in yashy_driver::outline::search(&model, &query) {
                println!("{line}");
            }
            exit_code(EXIT_OK)
        }
        Err(message) => {
            eprintln!("error: {message}");
            exit_code(EXIT_USAGE)
        }
    }
}

/// `yashyc context --task "..." [path] [--budget <n>] [--json]`: a
/// token-budgeted context pack.
fn context_cmd(args: &[String]) -> ExitCode {
    let mut json = false;
    let mut task: Option<String> = None;
    let mut focus: Option<String> = None;
    let mut mode: Option<yashy_driver::pack::PackMode> = None;
    let mut changed: Vec<String> = Vec::new();
    let mut diagnostic: Option<String> = None;
    let mut budget: usize = 8000;
    let mut input: Option<PathBuf> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--json" => json = true,
            "--mode" => {
                if i + 1 >= args.len() {
                    return usage_error("`--mode` needs a value");
                }
                match yashy_driver::pack::PackMode::parse(&args[i + 1]) {
                    Some(parsed) => mode = Some(parsed),
                    None => {
                        return usage_error("unknown pack mode; use focus, diff, tests, docs or error")
                    }
                }
                i += 1;
            }
            "--focus" => {
                if i + 1 >= args.len() {
                    return usage_error("`--focus` needs a value");
                }
                focus = Some(args[i + 1].clone());
                i += 1;
            }
            "--changed" => {
                if i + 1 >= args.len() {
                    return usage_error("`--changed` needs a value");
                }
                changed = args[i + 1]
                    .split(',')
                    .map(str::to_string)
                    .filter(|name| !name.is_empty())
                    .collect();
                i += 1;
            }
            "--diagnostic" => {
                if i + 1 >= args.len() {
                    return usage_error("`--diagnostic` needs a value");
                }
                diagnostic = Some(args[i + 1].clone());
                i += 1;
            }
            "--task" => {
                if i + 1 >= args.len() {
                    return usage_error("`--task` needs a value");
                }
                task = Some(args[i + 1].clone());
                i += 1;
            }
            "--budget" => {
                if i + 1 >= args.len() {
                    return usage_error("`--budget` needs a value");
                }
                match args[i + 1].parse::<usize>() {
                    Ok(parsed) => budget = parsed,
                    Err(_) => return usage_error("`--budget` needs a whole number"),
                }
                i += 1;
            }
            other if other.starts_with('-') => {
                return usage_error(&format!("unknown option `{other}` for `context`"))
            }
            other => {
                if input.is_some() {
                    return usage_error(&format!("unexpected extra argument `{other}`"));
                }
                input = Some(PathBuf::from(other));
            }
        }
        i += 1;
    }
    let Some(task) = task else {
        return usage_error("`context` needs `--task \"...\"`");
    };
    let root = input.unwrap_or_else(|| PathBuf::from("."));
    if let Some(mode) = mode {
        let request = yashy_driver::pack::PackRequest {
            focus: focus.unwrap_or_else(|| task.clone()),
            mode,
            budget,
            changed,
            diagnostic,
        };
        return match yashy_driver::pack::context_pack(&root, &request) {
            Ok(pack) => {
                if json {
                    println!("{}", pack.to_json().render());
                } else {
                    println!("# Pack: {} / {}", pack.mode.as_str(), pack.focus);
                    println!("revision {}", pack.revision);
                    for excerpt in &pack.excerpts {
                        println!(
                            "\n## {} ({}:{}..{})\n{}",
                            excerpt.reason, excerpt.file, excerpt.start, excerpt.end, excerpt.text
                        );
                    }
                    for dropped in &pack.dropped {
                        println!("dropped {}: {}", dropped.file, dropped.reason);
                    }
                }
                exit_code(EXIT_OK)
            }
            Err(message) => {
                eprintln!("error: {message}");
                exit_code(EXIT_USAGE)
            }
        };
    }
    match yashy_driver::context::build_context(&root, &task, budget, None) {
        Ok(pack) => {
            if json {
                println!("{}", pack.to_json().render());
            } else {
                print!("{}", pack.render_markdown());
            }
            exit_code(EXIT_OK)
        }
        Err(message) => {
            eprintln!("error: {message}");
            exit_code(EXIT_USAGE)
        }
    }
}

/// `yashyc mcp --root <dir>`: the MCP server on stdio.
fn review_command_args(args: &[String], command: &str) -> Result<(PathBuf, String, bool), ExitCode> {
    let mut root = PathBuf::from(".");
    let mut baseline: Option<String> = None;
    let mut json = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--json" => json = true,
            "--baseline" => {
                if index + 1 >= args.len() {
                    return Err(usage_error("`--baseline` needs an index JSON file"));
                }
                baseline = Some(args[index + 1].clone());
                index += 1;
            }
            other if other.starts_with('-') => {
                return Err(usage_error(&format!("unknown option `{other}` for `{command}`")))
            }
            other => root = PathBuf::from(other),
        }
        index += 1;
    }
    let Some(file) = baseline else {
        return Err(usage_error(&format!(
            "`{command}` needs `--baseline <index.json>`"
        )));
    };
    let text = match std::fs::read_to_string(&file) {
        Ok(text) => text,
        Err(error) => {
            eprintln!("error: {file}: {error}");
            return Err(exit_code(EXIT_USAGE));
        }
    };
    Ok((root, text, json))
}

fn review_cmd(args: &[String]) -> ExitCode {
    let (root, baseline, json) = match review_command_args(args, "review") {
        Ok(values) => values,
        Err(code) => return code,
    };
    match yashy_driver::review::review(&root, &baseline) {
        Ok(review) => {
            if json {
                println!("{}", review.to_json().render());
            } else {
                print!("{}", review.render_human());
            }
            exit_code(EXIT_OK)
        }
        Err(message) => {
            eprintln!("error: {message}");
            exit_code(EXIT_USAGE)
        }
    }
}

fn impact_cmd(args: &[String]) -> ExitCode {
    let (root, baseline, json) = match review_command_args(args, "impact") {
        Ok(values) => values,
        Err(code) => return code,
    };
    let review = match yashy_driver::review::review(&root, &baseline) {
        Ok(review) => review,
        Err(message) => {
            eprintln!("error: {message}");
            return exit_code(EXIT_USAGE);
        }
    };
    match yashy_driver::impact::impact(&root, &baseline, &review) {
        Ok(impact) => {
            if json {
                println!("{}", impact.to_json().render());
            } else {
                print!("{}", impact.render_human());
            }
            exit_code(EXIT_OK)
        }
        Err(message) => {
            eprintln!("error: {message}");
            exit_code(EXIT_USAGE)
        }
    }
}

fn advisory_cmd(args: &[String]) -> ExitCode {
    let mut feed_path: Option<String> = None;
    let mut package: Option<String> = None;
    let mut version: Option<String> = None;
    let mut keyring_path: Option<String> = None;
    let mut json = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--json" => json = true,
            "--check" | "--feed" | "--package" | "--version" | "--keyring" => {
                if index + 1 >= args.len() {
                    return usage_error(&format!("`{}` needs a value", args[index]));
                }
                let value = args[index + 1].clone();
                match args[index].as_str() {
                    "--check" | "--feed" => feed_path = Some(value),
                    "--package" => package = Some(value),
                    "--version" => version = Some(value),
                    "--keyring" => keyring_path = Some(value),
                    _ => unreachable!(),
                }
                index += 1;
            }
            other => return usage_error(&format!("unknown argument `{other}` for `advisory`")),
        }
        index += 1;
    }
    let Some(feed_path) = feed_path else {
        return usage_error("`advisory` needs `--check <feed.json>`");
    };
    let text = match std::fs::read_to_string(&feed_path) {
        Ok(text) => text,
        Err(error) => {
            eprintln!("error: {feed_path}: {error}");
            return exit_code(EXIT_USAGE);
        }
    };
    let feed = match yashy_driver::advisory::Feed::parse(&text) {
        Ok(feed) => feed,
        Err(message) => {
            eprintln!("error: {message}");
            return exit_code(EXIT_USAGE);
        }
    };
    if let Some(path) = &keyring_path {
        let keys = match yashy_driver::advisory::load_keyring(Path::new(path)) {
            Ok(keys) => keys,
            Err(message) => {
                eprintln!("error: {message}");
                return exit_code(EXIT_USAGE);
            }
        };
        if let Err(error) = feed.verify(&keys) {
            eprintln!("error: advisory feed signature: {error}");
            return exit_code(EXIT_COMPILE_ERROR);
        }
    }
    let (package, version) = match (package, version) {
        (Some(package), Some(version)) => (package, version),
        _ => {
            if json {
                println!("{}", feed.to_json().render());
            } else {
                println!("advisories: {}", feed.advisories.len());
                for advisory in &feed.advisories {
                    println!(
                        "  {} {} {} fixed in {}",
                        advisory.id, advisory.package, advisory.range, advisory.fixed
                    );
                }
            }
            return exit_code(EXIT_OK);
        }
    };
    match feed.matches(&package, &version) {
        Ok(matches) if matches.is_empty() => {
            if json {
                let mut root = Json::object();
                root.set("schema_version", Json::number(1));
                root.set("tool", Json::string("yashyc"));
                root.set("affected", Json::array(Vec::new()));
                println!("{}", root.render());
            } else {
                println!("advisories: none for {package} {version}");
            }
            exit_code(EXIT_OK)
        }
        Ok(matches) => {
            if json {
                let mut root = Json::object();
                root.set("schema_version", Json::number(1));
                root.set("tool", Json::string("yashyc"));
                root.set(
                    "affected",
                    Json::array(matches.iter().map(|advisory| advisory.to_json()).collect()),
                );
                println!("{}", root.render());
            } else {
                for advisory in matches {
                    eprintln!(
                        "error[{}]: {} {} is affected by {} ({}); fixed in {}",
                        advisory.severity,
                        package,
                        version,
                        advisory.id,
                        advisory.range,
                        advisory.fixed
                    );
                }
            }
            exit_code(EXIT_COMPILE_ERROR)
        }
        Err(message) => {
            eprintln!("error: {message}");
            exit_code(EXIT_USAGE)
        }
    }
}

fn conformance_cmd(args: &[String]) -> ExitCode {
    let mut root = PathBuf::from(".");
    let mut json = false;
    let mut fail_under: Option<u64> = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--json" => json = true,
            "--all" => {}
            "--fail-under" => {
                if index + 1 >= args.len() {
                    return usage_error("`--fail-under` needs a percentage");
                }
                match args[index + 1].parse::<u64>() {
                    Ok(parsed) if parsed <= 100 => fail_under = Some(parsed),
                    _ => return usage_error("`--fail-under` needs a percentage from 0 to 100"),
                }
                index += 1;
            }
            other if other.starts_with('-') => {
                return usage_error(&format!("unknown option `{other}` for `conformance`"))
            }
            other => root = PathBuf::from(other),
        }
        index += 1;
    }
    match yashy_driver::conformance::run(&root) {
        Ok(report) => {
            if json {
                println!("{}", report.to_json().render());
            } else {
                print!("{}", report.render_human());
            }
            let total = report.cases.len().max(1);
            let percent = (report.passed * 100 / total) as u64;
            if fail_under.is_some_and(|floor| percent < floor) {
                exit_code(EXIT_COMPILE_ERROR)
            } else {
                exit_code(EXIT_OK)
            }
        }
        Err(message) => {
            eprintln!("error: {message}");
            exit_code(EXIT_USAGE)
        }
    }
}

fn agent_plan(root: &Path, patch_file: &str) -> Result<Json, String> {
    use yashy_common::json_rpc::{self, Value};
    let text = std::fs::read_to_string(root.join(patch_file))
        .or_else(|_| std::fs::read_to_string(patch_file))
        .map_err(|error| format!("{patch_file}: {error}"))?;
    let patch = json_rpc::parse(&text).map_err(|error| format!("{patch_file}: {error}"))?;
    let mut files = Vec::new();
    let mut replacements = Vec::new();
    for edit in patch
        .get_path(&["edits"])
        .and_then(Value::as_array)
        .unwrap_or(&[])
    {
        let file = edit.get("file").and_then(Value::as_str).unwrap_or("");
        let start = edit
            .get_path(&["span", "start"])
            .and_then(Value::as_i64)
            .unwrap_or(0)
            .max(0) as usize;
        let end = edit
            .get_path(&["span", "end"])
            .and_then(Value::as_i64)
            .unwrap_or(0)
            .max(0) as usize;
        let replacement = edit
            .get("replacement")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let source = std::fs::read_to_string(root.join(file)).unwrap_or_default();
        let before = source.get(start..end.min(source.len())).unwrap_or("");
        replacements.push(replacement.clone());
        let mut entry = Json::object();
        entry.set("file", Json::string(file));
        entry.set("before", Json::string(before.trim()));
        entry.set("after", Json::string(replacement.trim()));
        entry.set(
            "lines_added",
            Json::number(replacement.lines().count() as i64),
        );
        entry.set("lines_removed", Json::number(before.lines().count() as i64));
        files.push(entry);
    }
    let touched = replacements.join("\n");
    let symbols: Vec<Json> = match yashy_driver::index::build_index(root) {
        Ok(model) => model
            .symbols
            .iter()
            .filter(|symbol| touched.contains(&symbol.name))
            .map(|symbol| Json::string(&symbol.name))
            .collect(),
        Err(_) => Vec::new(),
    };
    let capabilities: Vec<Json> = std::fs::read_to_string(root.join("yashy.toml"))
        .ok()
        .and_then(|text| yashy_manifest::Manifest::parse(&text).ok())
        .and_then(|manifest| manifest.capabilities)
        .map(|set| {
            set.entries()
                .iter()
                .map(|(name, _)| Json::string(*name))
                .collect()
        })
        .unwrap_or_default();
    let mut plan = Json::object();
    plan.set("schema_version", Json::number(1));
    plan.set("tool", Json::string("yashyc"));
    plan.set("verbs", Json::array(vec![Json::string("edit")]));
    plan.set("files", Json::array(files));
    plan.set("symbols", Json::array(symbols));
    plan.set("capabilities", Json::array(capabilities));
    Ok(plan)
}

fn agent_cmd(args: &[String]) -> ExitCode {
    let Some(subcommand) = args.first() else {
        return usage_error("`agent` needs a subcommand: `audit`, `policy`, `plan` or `approve`");
    };
    let mut json = false;
    let mut root = PathBuf::from(".");
    let mut tree: Option<String> = None;
    let mut patch: Option<String> = None;
    let mut verbs: Option<String> = None;
    let mut ttl: u64 = 300;
    let mut nonce: Option<String> = None;
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--json" => json = true,
            "--verify" => {}
            "--root" | "--tree" | "--patch" | "--verbs" | "--ttl" | "--nonce" => {
                if index + 1 >= args.len() {
                    return usage_error(&format!("`{}` needs a value", args[index]));
                }
                let value = args[index + 1].clone();
                match args[index].as_str() {
                    "--root" => root = PathBuf::from(value),
                    "--tree" => tree = Some(value),
                    "--patch" => patch = Some(value),
                    "--verbs" => verbs = Some(value),
                    "--ttl" => match value.parse::<u64>() {
                        Ok(parsed) => ttl = parsed,
                        Err(_) => return usage_error("`--ttl` needs a whole number of seconds"),
                    },
                    "--nonce" => nonce = Some(value),
                    _ => unreachable!(),
                }
                index += 1;
            }
            other if other.starts_with('-') => {
                return usage_error(&format!("unknown option `{other}` for `agent`"))
            }
            other => return usage_error(&format!("unexpected extra argument `{other}`")),
        }
        index += 1;
    }
    match subcommand.as_str() {
        "audit" => {
            let verification = yashy_driver::agent::AuditLog::verify(&root);
            if json {
                println!("{}", verification.to_json().render());
            } else if verification.ok {
                println!(
                    "{} entries  chain ok  first bad: none",
                    verification.entries
                );
            } else {
                match verification.first_bad_seq {
                    Some(seq) => eprintln!(
                        "error[{}]: {}  first bad seq: {seq}",
                        yashy_driver::agent::AUDIT_FAILURE,
                        verification.message
                    ),
                    None => eprintln!(
                        "error[{}]: {}",
                        yashy_driver::agent::AUDIT_FAILURE,
                        verification.message
                    ),
                }
            }
            if verification.ok {
                exit_code(EXIT_OK)
            } else {
                exit_code(EXIT_COMPILE_ERROR)
            }
        }
        "policy" => match yashy_driver::agent::AgentPolicy::load(&root) {
            Ok(policy) => {
                if json {
                    println!("{}", policy.to_json().render());
                } else {
                    println!("allow: {}", policy.allow.join(", "));
                    println!(
                        "protected: {}",
                        if policy.protected_paths.is_empty() {
                            "none".to_string()
                        } else {
                            policy.protected_paths.join(", ")
                        }
                    );
                    println!(
                        "max edits: {}",
                        policy
                            .max_edits
                            .map_or("none".to_string(), |limit| limit.to_string())
                    );
                    println!("audit max bytes: {}", policy.audit_max_bytes);
                    println!(
                        "require approval: {}",
                        if policy.require_approval.is_empty() {
                            "none".to_string()
                        } else {
                            policy.require_approval.join(", ")
                        }
                    );
                }
                exit_code(EXIT_OK)
            }
            Err(message) => {
                eprintln!("error: {message}");
                exit_code(EXIT_USAGE)
            }
        },
        "plan" => {
            let Some(patch_file) = patch else {
                return usage_error("`plan` needs a patch file: `--patch <file>`");
            };
            match agent_plan(&root, &patch_file) {
                Ok(plan) => {
                    if json {
                        println!("{}", plan.render());
                    } else {
                        let files = plan.get("files").map_or(0, |value| match value {
                            Json::Array(items) => items.len(),
                            _ => 0,
                        });
                        println!("plan: {files} files, verbs {}", "edit");
                        if let Some(Json::Array(items)) = plan.get("symbols") {
                            let names: Vec<String> = items
                                .iter()
                                .filter_map(|item| match item {
                                    Json::String(name) => Some(name.clone()),
                                    _ => None,
                                })
                                .collect();
                            println!("symbols: {}", names.join(", "));
                        }
                    }
                    exit_code(EXIT_OK)
                }
                Err(message) => {
                    eprintln!("error: {message}");
                    exit_code(EXIT_USAGE)
                }
            }
        }
        "approve" => {
            let Some(patch_file) = patch else {
                return usage_error("`approve` needs a patch file: `--patch <file>`");
            };
            let patch_text = match std::fs::read_to_string(root.join(&patch_file))
                .or_else(|_| std::fs::read_to_string(&patch_file))
            {
                Ok(text) => text,
                Err(error) => {
                    eprintln!("error: {patch_file}: {error}");
                    return exit_code(EXIT_USAGE);
                }
            };
            let patch_value = match yashy_common::json_rpc::parse(&patch_text) {
                Ok(value) => value,
                Err(error) => {
                    eprintln!("error: {patch_file}: {error}");
                    return exit_code(EXIT_USAGE);
                }
            };
            let tree = match tree {
                Some(tree) => tree,
                None => match yashy_driver::edit::tree_hash(&root) {
                    Ok(tree) => tree,
                    Err(error) => {
                        eprintln!("error: {}", error.message());
                        return exit_code(EXIT_USAGE);
                    }
                },
            };
            let names: Vec<String> = verbs
                .unwrap_or_else(|| "edit".to_string())
                .split(',')
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_string)
                .collect();
            let nonce = nonce.unwrap_or_else(|| yashy_driver::approval::now_unix_ms().to_string());
            match yashy_driver::approval::mint(&root, &tree, &patch_value, names, ttl, &nonce) {
                Ok((token, approval)) => {
                    if json {
                        println!("{}", approval.to_json(&token).render());
                    } else {
                        println!("approved {token}");
                        println!("tree {}", approval.tree_hash);
                        println!("patch {}", approval.patch_hash);
                        println!("verbs {}", approval.verbs.join(", "));
                        println!("expires {}", approval.expires_unix_ms);
                    }
                    exit_code(EXIT_OK)
                }
                Err(message) => {
                    eprintln!("error: {message}");
                    exit_code(EXIT_USAGE)
                }
            }
        }
        other => usage_error(&format!("unknown `agent` subcommand `{other}`")),
    }
}

fn mcp_cmd(args: &[String]) -> ExitCode {
    let mut root = PathBuf::from(".");
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--root" => {
                if i + 1 >= args.len() {
                    return usage_error("`--root` needs a value");
                }
                root = PathBuf::from(&args[i + 1]);
                i += 1;
            }
            other if other.starts_with('-') => {
                return usage_error(&format!("unknown option `{other}` for `mcp`"))
            }
            other => {
                return usage_error(&format!("unexpected extra argument `{other}`"));
            }
        }
        i += 1;
    }
    match yashy_driver::mcp::serve(&root) {
        Ok(()) => exit_code(EXIT_OK),
        Err(error) => {
            eprintln!("error: {error}");
            exit_code(EXIT_INTERNAL)
        }
    }
}

/// `yashyc lsp`: run the language server. The server is a sibling binary
/// (`yashy-lsp` beside this executable): the driver cannot link the LSP
/// crate (`yashy-lsp` depends on `yashy-driver`), so this is an exec, not
/// a reimplementation. A missing sibling exits 2 naming the path. The
/// lookup is one function because `yashyc doctor` says what it found; the
/// environment override exists so a test can point it at a stub, never
/// as a user-facing setting.
fn lsp_sibling() -> PathBuf {
    if let Some(override_path) = env::var_os("YASHY_LSP_SIBLING") {
        if !override_path.is_empty() {
            return PathBuf::from(override_path);
        }
    }
    std::env::current_exe()
        .ok()
        .and_then(|exe| {
            exe.parent()
                .map(|dir| dir.join(format!("yashy-lsp{}", std::env::consts::EXE_SUFFIX)))
        })
        .unwrap_or_else(|| PathBuf::from("yashy-lsp"))
}

fn lsp_cmd(args: &[String]) -> ExitCode {
    let sibling = lsp_sibling();
    if args.iter().any(|arg| arg == "--version") {
        let driver = env!("CARGO_PKG_VERSION");
        println!("yashyc {driver}");
        match Command::new(&sibling).arg("--version").output() {
            Ok(output) => {
                let server = String::from_utf8_lossy(&output.stdout).trim().to_string();
                println!("{server}");
                let server_version = server.rsplit(' ').next().unwrap_or("");
                if server_version != driver {
                    eprintln!(
                        "warning: server version {server_version} does not match driver {driver}"
                    );
                }
            }
            Err(error) => {
                eprintln!(
                    "error: cannot run the language server at {}: {error}.",
                    sibling.display()
                );
                eprintln!("install it with your usual package manager, or rebuild the workspace.");
                return exit_code(EXIT_USAGE);
            }
        }
        return exit_code(EXIT_OK);
    }
    for arg in args {
        if arg.starts_with('-') {
            return usage_error(&format!("unknown option `{arg}` for `lsp`"));
        }
        return usage_error(&format!("unexpected extra argument `{arg}`"));
    }
    if !sibling.is_file() {
        eprintln!(
            "error: the language server binary is missing at {}.",
            sibling.display()
        );
        eprintln!("install it with your usual package manager, or rebuild the workspace.");
        return exit_code(EXIT_USAGE);
    }
    let mut child = match Command::new(&sibling)
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            eprintln!(
                "error: cannot run the language server at {}: {error}.",
                sibling.display()
            );
            return exit_code(EXIT_USAGE);
        }
    };
    match child.wait() {
        Ok(status) => exit_code(exit_status_code(status)),
        Err(error) => {
            eprintln!("error: cannot wait for the language server: {error}.");
            exit_code(EXIT_INTERNAL)
        }
    }
}

/// Build one program with the event trace armed, run it, and render every
/// allocation site and the blocks still live at exit. A program the verifier
/// proves cannot leak is still useful here (the sites of freed blocks are
/// real), and `--no-verify` lets a deliberately leaking program be measured;
/// the build stamps the usual unverified sidecar in that case.
fn alloc_report(input: PathBuf, no_verify: bool) -> ExitCode {
    use yashy_driver::trace;

    let dir = temp_run_dir();
    if let Err(error) = fs::create_dir_all(&dir) {
        eprintln!("error: cannot create {}: {error}", dir.display());
        return exit_code(EXIT_COMPILE_ERROR);
    }
    let output = dir.join(format!("alloc{}", std::env::consts::EXE_SUFFIX));
    let events_file = dir.join("events.txt");
    let mut build_arguments = vec![
        input.to_string_lossy().into_owned(),
        "-o".to_string(),
        output.to_string_lossy().into_owned(),
        "--debug-info".to_string(),
    ];
    if no_verify {
        build_arguments.push("--no-verify".to_string());
    }
    let code = build(CommandKind::Run, &build_arguments);
    if code != exit_code(EXIT_OK) {
        let _ = fs::remove_dir_all(&dir);
        return code;
    }
    let mut ran = false;
    for attempt in 0..LAUNCH_ATTEMPTS {
        let candidate = match launch_candidate(&output, attempt) {
            Ok(path) => path,
            Err(error) => {
                eprintln!("error: cannot prepare the program: {error}");
                let _ = fs::remove_dir_all(&dir);
                return exit_code(EXIT_INTERNAL);
            }
        };
        let child = Command::new(&candidate)
            .env("YASHY_TRACE_FILE", &events_file)
            .stdout(Stdio::null())
            .status();
        match child {
            Ok(_) => {
                ran = true;
                break;
            }
            Err(error) if blocked_by_policy(&error) => continue,
            Err(error) => {
                eprintln!("error: cannot run the program: {error}");
                let _ = fs::remove_dir_all(&dir);
                return exit_code(EXIT_INTERNAL);
            }
        }
    }
    if !ran {
        let _ = fs::remove_dir_all(&dir);
        return exit_code(EXIT_INTERNAL);
    }
    let text = match fs::read_to_string(&events_file) {
        Ok(text) => text,
        Err(error) => {
            eprintln!("error: cannot read {}: {error}", events_file.display());
            let _ = fs::remove_dir_all(&dir);
            return exit_code(EXIT_INTERNAL);
        }
    };

    let (mut events, dropped) = trace::parse_events(&text);
    // The symbolizer reads the binary, so the scratch directory has to stay
    // until addresses have been turned into source spans.
    trace::symbolize(&mut events, &output);
    let _ = fs::remove_dir_all(&dir);

    // Walk in table order: an allocation is live if its pointer has not been
    // freed by the time the program ends. Reusing a pointer after a free is
    // common (a loop that allocates and releases one buffer), so a "was ever
    // freed" set would wrongly call the second allocation a leak.
    let mut live: std::collections::BTreeMap<u64, &trace::Event> =
        std::collections::BTreeMap::new();
    let mut frees = std::collections::HashSet::new();
    for event in &events {
        match event.kind {
            "alloc" | "alloc_uninit" => {
                live.insert(event.ptr, event);
                frees.remove(&event.ptr);
            }
            "free" => {
                if live.remove(&event.ptr).is_none() {
                    frees.insert(event.ptr);
                }
            }
            _ => {}
        }
    }
    #[derive(Default)]
    struct SiteFacts {
        count: u64,
        bytes: i64,
        classes: std::collections::BTreeSet<String>,
        paths: std::collections::BTreeSet<String>,
        unsafe_: bool,
    }

    /// A display bucket, not a claim about the allocator's own size classes.
    fn size_class(bytes: i64) -> String {
        if bytes > 4096 {
            return "large".to_string();
        }
        let mut class = 8i64;
        while class < bytes {
            class *= 2;
        }
        class.to_string()
    }

    fn record_site<'a>(
        map: &mut std::collections::BTreeMap<&'a str, SiteFacts>,
        event: &'a trace::Event,
    ) {
        let entry = map.entry(event.span.as_str()).or_default();
        entry.count += 1;
        entry.bytes += event.size;
        entry.unsafe_ |= event.kind == "alloc_uninit";
        entry.classes.insert(size_class(event.size));
        let path = if event.stack.is_empty() {
            "?".to_string()
        } else {
            event.stack.join(" <- ")
        };
        entry.paths.insert(path);
    }

    let mut by_site: std::collections::BTreeMap<&str, SiteFacts> =
        std::collections::BTreeMap::new();
    for event in live.values() {
        record_site(&mut by_site, event);
    }
    let mut all_by_site: std::collections::BTreeMap<&str, SiteFacts> =
        std::collections::BTreeMap::new();
    for event in events
        .iter()
        .filter(|event| matches!(event.kind, "alloc" | "alloc_uninit"))
    {
        record_site(&mut all_by_site, event);
    }
    let render_sites = |map: std::collections::BTreeMap<&str, SiteFacts>| -> Vec<Json> {
        map.into_iter()
            .map(|(span, facts)| {
                let mut site = Json::object();
                site.set("span", Json::string(span));
                site.set("count", Json::number(facts.count as i64));
                site.set("bytes", Json::number(facts.bytes));
                site.set("unsafe", Json::bool(facts.unsafe_));
                site.set(
                    "size_class",
                    Json::array(facts.classes.into_iter().map(Json::string).collect()),
                );
                site.set(
                    "call_path",
                    Json::array(facts.paths.into_iter().map(Json::string).collect()),
                );
                site
            })
            .collect()
    };
    let allocations = render_sites(all_by_site);
    let sites = render_sites(by_site);

    let mut report = Json::object();
    report.set("schema_version", Json::number(1));
    report.set("tool", Json::string("profile --alloc-report"));
    report.set(
        "program",
        Json::string(input.to_string_lossy().into_owned()),
    );
    report.set("live_allocations", Json::number(live.len() as i64));
    report.set("events", Json::number(events.len() as i64));
    report.set("dropped", Json::number(dropped as i64));
    report.set("allocations", Json::array(allocations));
    report.set("sites", Json::array(sites));
    println!("{}", report.render());
    exit_code(EXIT_OK)
}

/// `yashyc coverage <file.ya>` or `yashyc coverage --tests [<dir>]`
///
/// The debug info from HIR spans is what makes the result source-based: clang
/// writes a coverage mapping into the instrumented binary and llvm-cov reads
/// it back as lcov records naming Yashy lines, not LLVM instructions.
///
/// With `--tests`, every test under the directory is instrumented in its own
/// binary; the raw profiles are merged into one `profdata` and every binary is
/// exported together, so the report describes the suite as one run. This is
/// the shape a CI gate needs: a line is covered when any test reached it.
fn coverage_cmd(args: &[String]) -> ExitCode {
    use yashy_driver::coverage;

    let mut json = false;
    let mut format: Option<String> = None;
    let mut fail_under: Option<f64> = None;
    let mut source: Option<PathBuf> = None;
    let mut program_args: Vec<String> = Vec::new();
    let mut after_separator = false;
    let mut tests = false;
    let mut filter: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        if after_separator {
            program_args.push(args[i].clone());
            i += 1;
            continue;
        }
        match args[i].as_str() {
            "--json" => json = true,
            "--tests" => tests = true,
            "--filter" => {
                if i + 1 < args.len() {
                    filter = Some(args[i + 1].clone());
                    i += 1;
                } else {
                    return usage_error("`--filter` needs a substring");
                }
            }
            "--format" => {
                if i + 1 < args.len() {
                    format = Some(args[i + 1].clone());
                    i += 1;
                } else {
                    return usage_error("`--format` needs `lcov`, `json` or `sarif`");
                }
            }
            "--fail-under" => {
                if i + 1 < args.len() {
                    match args[i + 1].parse::<f64>() {
                        Ok(value) if (0.0..=100.0).contains(&value) => fail_under = Some(value),
                        _ => return usage_error("`--fail-under` needs a percentage from 0 to 100"),
                    }
                    i += 1;
                } else {
                    return usage_error("`--fail-under` needs a percentage");
                }
            }
            "--" => after_separator = true,
            other if other.starts_with('-') => {
                return usage_error(&format!("unknown option `{other}` for `coverage`"));
            }
            other => {
                if source.is_some() {
                    return usage_error("`coverage` takes one source file or one test directory");
                }
                source = Some(PathBuf::from(other));
            }
        }
        i += 1;
    }
    if !tests && filter.is_some() {
        return usage_error("`--filter` needs `--tests`");
    }
    if tests && !program_args.is_empty() {
        return usage_error("`--tests` runs each test without arguments; drop the `-- args`");
    }
    let Some(source) = source.or_else(|| tests.then(|| PathBuf::from("tests"))) else {
        return usage_error("`coverage` needs a source file or `--tests [<dir>]`");
    };
    match format.as_deref() {
        None | Some("lcov") | Some("json") | Some("sarif") => {}
        Some(other) => return usage_error(&format!("unknown coverage format `{other}`")),
    }

    let toolchain = match detect() {
        Ok(toolchain) => toolchain,
        Err(message) => {
            eprintln!("error: {message}");
            return exit_code(EXIT_COMPILE_ERROR);
        }
    };
    let Some(profdata) =
        coverage::find_tool("llvm-profdata", &toolchain.llc, "YASHY_LLVM_PROFDATA")
    else {
        eprintln!("error: `llvm-profdata` is needed for coverage; set YASHY_LLVM_PROFDATA or install LLVM");
        return exit_code(EXIT_USAGE);
    };
    let Some(llvm_cov) = coverage::find_tool("llvm-cov", &toolchain.llc, "YASHY_LLVM_COV") else {
        eprintln!("error: `llvm-cov` is needed for coverage; set YASHY_LLVM_COV or install LLVM");
        return exit_code(EXIT_USAGE);
    };

    let dir = temp_run_dir();
    if let Err(error) = fs::create_dir_all(&dir) {
        eprintln!("error: cannot create {}: {error}", dir.display());
        return exit_code(EXIT_INTERNAL);
    }

    // Every instrumented binary and its raw profile. The single-file mode has
    // one of each; `--tests` has one per test, merged below.
    let mut binaries: Vec<PathBuf> = Vec::new();
    let mut raws: Vec<PathBuf> = Vec::new();
    let mut build_sources: Vec<PathBuf> = vec![source.clone()];
    if tests {
        build_sources.clear();
        let mut discovered = Vec::new();
        if let Err(error) = collect_tests(&source, &mut discovered) {
            eprintln!("error: cannot read {}: {error}", source.display());
            let _ = fs::remove_dir_all(&dir);
            return exit_code(EXIT_USAGE);
        }
        discovered.sort();
        if let Some(filter) = &filter {
            discovered.retain(|path| {
                path.to_string_lossy().contains(filter) || display_test_path(path).contains(filter)
            });
        }
        if discovered.is_empty() {
            eprintln!("error: no tests under {}", source.display());
            let _ = fs::remove_dir_all(&dir);
            return exit_code(EXIT_USAGE);
        }
        build_sources = discovered;
    }

    for (index, test) in build_sources.iter().enumerate() {
        let binary = dir.join(format!("coverage-{index}{}", std::env::consts::EXE_SUFFIX));
        let build_args = vec![
            test.to_string_lossy().into_owned(),
            "-o".to_string(),
            binary.to_string_lossy().into_owned(),
            "--debug-info".to_string(),
        ];
        env::set_var("YASHY_COVERAGE", "1");
        let built = build(CommandKind::Run, &build_args);
        env::remove_var("YASHY_COVERAGE");
        if built != exit_code(EXIT_OK) {
            let _ = fs::remove_dir_all(&dir);
            return built;
        }
        let raw = dir.join(format!("coverage-{index}.profraw"));
        let mut child = Command::new(&binary);
        child.env("LLVM_PROFILE_FILE", &raw).stdout(Stdio::null());
        if !tests {
            for argument in &program_args {
                child.arg(argument);
            }
        }
        if !child.status().is_ok_and(|status| status.success()) {
            if tests {
                eprintln!(
                    "error: test {} did not exit successfully",
                    display_test_path(test)
                );
            } else {
                eprintln!("error: the instrumented program did not exit successfully");
            }
            let _ = fs::remove_dir_all(&dir);
            return exit_code(EXIT_COMPILE_ERROR);
        }
        binaries.push(binary);
        raws.push(raw);
    }

    // One merged profile, whether the run had one binary or a hundred.
    let merged = dir.join("coverage.profdata");
    let mut merge = Command::new(&profdata);
    merge.arg("merge").arg("-sparse");
    for raw in &raws {
        merge.arg(raw);
    }
    merge.arg("-o").arg(&merged);
    if !merge.output().is_ok_and(|result| result.status.success()) {
        eprintln!("error: `llvm-profdata merge` failed; no coverage data was produced");
        let _ = fs::remove_dir_all(&dir);
        return exit_code(EXIT_INTERNAL);
    }
    // Every object contributes its own coverage mapping; llvm-cov exports them
    // against the one merged profile, so a line reached by any test is covered.
    let mut export = Command::new(&llvm_cov);
    export.arg("export");
    for binary in &binaries {
        export.arg(binary);
    }
    export
        .arg(format!("-instr-profile={}", merged.display()))
        .arg("-format=lcov");
    let exported = match export.output() {
        Ok(result) if result.status.success() => result,
        Ok(result) => {
            eprintln!(
                "error: `llvm-cov export` failed: {}",
                String::from_utf8_lossy(&result.stderr)
            );
            let _ = fs::remove_dir_all(&dir);
            return exit_code(EXIT_INTERNAL);
        }
        Err(error) => {
            eprintln!("error: cannot run llvm-cov: {error}");
            let _ = fs::remove_dir_all(&dir);
            return exit_code(EXIT_INTERNAL);
        }
    };
    let lcov = String::from_utf8_lossy(&exported.stdout).into_owned();
    let files = coverage::parse_lcov(&lcov);
    let (covered, total) = coverage::summary(&files);
    let percent = if total == 0 {
        0.0
    } else {
        covered as f64 * 100.0 / total as f64
    };
    if format.as_deref() == Some("sarif") {
        println!("{}", coverage::to_sarif(&files).render());
    } else if json || format.as_deref() == Some("json") {
        println!("{}", coverage::to_json(&files, &source).render());
    } else {
        print!("{lcov}");
    }
    let result = match fail_under {
        Some(minimum) if percent < minimum => {
            eprintln!(
                "error: coverage is {percent:.2}% (below {minimum:.2}%): {covered}/{total} lines"
            );
            EXIT_COMPILE_ERROR
        }
        _ => EXIT_OK,
    };
    let _ = fs::remove_dir_all(&dir);
    exit_code(result)
}

fn profile_cmd(args: &[String]) -> ExitCode {
    let sample = args.iter().any(|arg| arg == "--sample");
    let json = args.iter().any(|arg| arg == "--json");
    let no_verify = args.iter().any(|arg| arg == "--no-verify");
    // `--alloc-report <file>` runs a program with the runtime's trace armed and
    // renders every allocation site and the blocks still live at exit as a
    // schema-v1 table. The site comes from the runtime's recorded caller, not
    // from a guess. `--no-verify` is the one way to measure a program the
    // verifier would refuse for leaking, which is the case a leak report is
    // for.
    if let Some(position) = args.iter().position(|arg| arg == "--alloc-report") {
        let Some(path) = args.get(position + 1) else {
            return usage_error("`--alloc-report` needs a file");
        };
        return alloc_report(PathBuf::from(path), no_verify);
    }
    // `--report <json>` renders a recorded table; without it, a fresh
    // measurement runs over the built program.
    if let Some(position) = args.iter().position(|arg| arg == "--report") {
        let Some(path) = args.get(position + 1) else {
            return usage_error("`--report` needs a file");
        };
        match std::fs::read_to_string(path) {
            Ok(text) => match yashy_driver::profile::parse_report(&text) {
                Ok(report) => {
                    if json {
                        println!("{}", yashy_common::json_rpc::to_string(&report.to_json()));
                    } else {
                        print!("{}", report.render());
                    }
                    return exit_code(EXIT_OK);
                }
                Err(message) => {
                    eprintln!("error: {message}");
                    return exit_code(EXIT_USAGE);
                }
            },
            Err(error) => {
                eprintln!("error: cannot read {path}: {error}");
                return exit_code(EXIT_USAGE);
            }
        }
    }
    let mut input: Option<PathBuf> = None;
    for arg in args {
        match arg.as_str() {
            "--sample" | "--json" | "--no-verify" => {}
            other if other.starts_with('-') => {
                return usage_error(&format!("unknown option `{other}` for `profile`"))
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
        return usage_error("`profile` needs a source file");
    };
    let dir = temp_run_dir();
    if let Err(error) = fs::create_dir_all(&dir) {
        eprintln!("error: cannot create {}: {error}", dir.display());
        return exit_code(EXIT_COMPILE_ERROR);
    }
    let output = dir.join(format!("profile{}", std::env::consts::EXE_SUFFIX));
    let mut build_arguments = vec![
        input.to_string_lossy().into_owned(),
        "-o".to_string(),
        output.to_string_lossy().into_owned(),
    ];
    if sample {
        // The sampler's addresses are only useful with the debug info that
        // names the function and source line they fall in. `-O0` keeps the
        // source functions distinct: at `-O2` a hot helper is inlined into its
        // caller and the sample would be attributed to a function that is not
        // in the source at all.
        build_arguments.push("--debug-info".to_string());
        build_arguments.push("-O0".to_string());
        if no_verify {
            build_arguments.push("--no-verify".to_string());
        }
    }
    let code = build(CommandKind::Run, &build_arguments);
    if code != exit_code(EXIT_OK) {
        let _ = fs::remove_dir_all(&dir);
        return code;
    }
    if sample {
        let sample_file = dir.join("samples.txt");
        let sample_path = sample_file.to_string_lossy().into_owned();
        let envs = [
            ("YASHY_SAMPLE_FILE", sample_path.as_str()),
            ("YASHY_SAMPLE_INTERVAL_US", "1000"),
        ];
        let status = status_with_retry_quiet(&output, &[], &envs);
        match status {
            Ok(status) if status.success() => {
                let parsed = std::fs::read_to_string(&sample_file)
                    .ok()
                    .map(|text| yashy_driver::sample::parse(&text))
                    .unwrap_or_default();
                if !parsed.addresses.is_empty() {
                    let report =
                        yashy_driver::sample::measure(&input.to_string_lossy(), &parsed, &output);
                    if json {
                        println!("{}", yashy_common::json_rpc::to_string(&report.to_json()));
                    } else {
                        print!("{}", report.render());
                    }
                    if report.hot().is_none() {
                        eprintln!(
                            "note: no function held 80% of the samples; the rows are still the measured shares."
                        );
                    }
                    let _ = fs::remove_dir_all(&dir);
                    return exit_code(EXIT_OK);
                }
                eprintln!(
                    "note: the native sampler recorded no samples; measuring the counter path instead"
                );
            }
            Ok(status) => {
                let _ = fs::remove_dir_all(&dir);
                return exit_code(exit_status_code(status));
            }
            Err(error) => {
                let _ = fs::remove_dir_all(&dir);
                eprintln!("error: cannot run {}: {error}", output.display());
                return exit_code(EXIT_COMPILE_ERROR);
            }
        }
    }
    // Measure honestly: build, run, and report the wall time. The per-function
    // table comes from a recorded `--report`; a fresh run has no counters to
    // aggregate, so the report says so instead of inventing rows.
    let started = std::time::Instant::now();
    let status = status_with_retry_quiet(&output, &[], &[]);
    let elapsed = started.elapsed();
    let _ = fs::remove_dir_all(&dir);
    match status {
        Ok(status) if status.success() => {
            let report = yashy_driver::profile::Report {
                program: input.to_string_lossy().into_owned(),
                overhead_pct: 0.0,
                functions: vec![yashy_driver::profile::FunctionRow {
                    name: "main (wall clock; see --report for per-function)".to_string(),
                    calls: 1,
                    self_ns: elapsed.as_nanos() as u64,
                    total_ns: elapsed.as_nanos() as u64,
                }],
            };
            if json {
                println!("{}", yashy_common::json_rpc::to_string(&report.to_json()));
            } else {
                print!("{}", report.render());
            }
            println!(
                "note: a fresh run carries no per-function counters; record one with instrumentation and render it with `yashyc profile --report`."
            );
            exit_code(EXIT_OK)
        }
        Ok(status) => exit_code(exit_status_code(status)),
        Err(error) => {
            eprintln!("error: cannot run {}: {error}", output.display());
            exit_code(EXIT_COMPILE_ERROR)
        }
    }
}

/// `yashyc doc [path] [--out DIR] [--json] [--private]`: render the public
/// surface as Markdown (or the same model as JSON).
fn doc_cmd(args: &[String]) -> ExitCode {
    let mut json = false;
    let mut private = false;
    let mut serve = false;
    let mut port = 0u16;
    let mut out_dir: Option<PathBuf> = None;
    let mut versioned = false;
    let mut version: Option<String> = None;
    let mut input: Option<PathBuf> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--json" => json = true,
            "--private" => private = true,
            "--serve" => serve = true,
            "--port" => {
                if i + 1 >= args.len() {
                    return usage_error("`--port` needs a value");
                }
                port = match args[i + 1].parse::<u16>() {
                    Ok(port) => port,
                    Err(_) => return usage_error("`--port` needs a number from 0 to 65535"),
                };
                i += 1;
            }
            "--versioned" => versioned = true,
            "--version" => {
                if i + 1 >= args.len() {
                    return usage_error("`--version` needs a value");
                }
                version = Some(args[i + 1].clone());
                i += 1;
            }
            "--out" => {
                if i + 1 >= args.len() {
                    return usage_error("`--out` needs a value");
                }
                out_dir = Some(PathBuf::from(&args[i + 1]));
                i += 1;
            }
            other if other.starts_with('-') => {
                return usage_error(&format!("unknown option `{other}` for `doc`"))
            }
            other => {
                if input.is_some() {
                    return usage_error(&format!("unexpected extra argument `{other}`"));
                }
                input = Some(PathBuf::from(other));
            }
        }
        i += 1;
    }
    let root = input.unwrap_or_else(|| PathBuf::from("."));
    if versioned {
        if json {
            return usage_error("`--versioned` writes files; `--json` is not a file write");
        }
        let model = match yashy_driver::doc::build_doc(&root, private) {
            Ok(model) => model,
            Err(message) => {
                eprintln!("error: {message}");
                return exit_code(EXIT_USAGE);
            }
        };
        let label = version.unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string());
        match yashy_driver::versioned::write(&root, &label, &model) {
            Ok(report) => {
                println!(
                    "wrote version {} ({} modules) to {}",
                    report.version,
                    report.modules,
                    report.directory.display()
                );
                return exit_code(EXIT_OK);
            }
            Err(message) => {
                eprintln!("error: {message}");
                return exit_code(EXIT_COMPILE_ERROR);
            }
        }
    }
    if serve {
        match yashy_driver::doc_server::bind(&root, private, port) {
            Ok((listener, model, chosen)) => {
                println!("serving docs on http://127.0.0.1:{chosen}/ (read-only)");
                use std::io::Write as _;
                let _ = std::io::stdout().flush();
                yashy_driver::doc_server::serve_bound(listener, model);
                // `serve` blocks; reaching here would mean the loopback listener
                // closed, which is a clean exit for a docs server.
                exit_code(EXIT_OK)
            }
            Err(message) => {
                eprintln!("error: {message}");
                exit_code(EXIT_USAGE)
            }
        }
    } else {
        match yashy_driver::doc::build_doc(&root, private) {
            Ok(model) => {
                if json {
                    println!("{}", yashy_driver::doc::to_json(&model).render());
                } else {
                    let out = out_dir.unwrap_or_else(|| PathBuf::from("target/doc"));
                    if let Err(error) = model.write_to(&out) {
                        eprintln!("error: cannot write docs to {}: {error}", out.display());
                        return exit_code(EXIT_COMPILE_ERROR);
                    }
                    println!("wrote {} modules to {}", model.modules.len(), out.display());
                }
                exit_code(EXIT_OK)
            }
            Err(message) => {
                eprintln!("error: {message}");
                exit_code(EXIT_USAGE)
            }
        }
    }
}

fn print_expand_json(sources: &SourceMap, file: FileId, map: &yashy_expand::ExpansionMap) {
    let mut report = Json::object();
    report.set("schema_version", Json::number(1));
    report.set("tool", Json::string("yashyc"));
    report.set(
        "expansions",
        Json::array(
            map.records
                .iter()
                .map(|record| {
                    let mut entry = Json::object();
                    entry.set("macro", Json::string(&record.macro_name));
                    entry.set("arm_span", span_json(sources, file, record.arm_span));
                    entry.set(
                        "invocation_span",
                        span_json(sources, file, record.invocation_span),
                    );
                    entry.set("output_tokens", Json::number(record.output_tokens as i64));
                    entry
                })
                .collect(),
        ),
    );
    println!("{}", report.render());
}

fn render_tokens(tokens: &[yashy_lexer::Token], sources: &SourceMap, file: FileId) -> String {
    let mut out = String::new();
    for token in tokens {
        if token.kind == yashy_lexer::TokenKind::Eof {
            continue;
        }
        let text = match token.kind {
            yashy_lexer::TokenKind::Ident
            | yashy_lexer::TokenKind::I64
            | yashy_lexer::TokenKind::F64
            | yashy_lexer::TokenKind::Str
            | yashy_lexer::TokenKind::Char
            | yashy_lexer::TokenKind::DocComment => sources.snippet(file, token.span).to_string(),
            _ => token.kind.spelling().unwrap_or("").to_string(),
        };
        if text.is_empty() {
            continue;
        }
        if !out.is_empty() && !out.ends_with('\n') {
            out.push(' ');
        }
        out.push_str(&text);
    }
    out.push('\n');
    out
}

/// A failure from `yashyc lock --check` or the build-time check.
enum LockCheckError {
    /// The command cannot run: no manifest, wrong arguments.
    Usage(String),
    /// The files disagree. Carries the code the diagnostic prints.
    Diagnostic { code: &'static str, message: String },
}

impl LockCheckError {
    fn report(&self) {
        match self {
            LockCheckError::Usage(message) => eprintln!("error: {message}"),
            LockCheckError::Diagnostic { code, message } => {
                eprintln!("error[{code}]: {message}");
                eprintln!("  docs: {}", codes::docs_url(code));
            }
        }
    }

    fn status(&self) -> ExitCode {
        match self {
            LockCheckError::Usage(_) => exit_code(EXIT_USAGE),
            LockCheckError::Diagnostic { .. } => exit_code(EXIT_COMPILE_ERROR),
        }
    }
}

/// `yashyc lock --check [dir]`.
fn lock_cmd(args: &[String]) -> ExitCode {
    let mut check = false;
    let mut directory: Option<PathBuf> = None;
    for arg in args {
        match arg.as_str() {
            "--check" => check = true,
            other if other.starts_with('-') => {
                return usage_error(&format!("unknown option `{other}` for `lock`"))
            }
            other => {
                if directory.is_some() {
                    return usage_error(&format!("unexpected extra argument `{other}`"));
                }
                directory = Some(PathBuf::from(other));
            }
        }
    }
    if !check {
        return usage_error("`lock` needs `--check`; `yashyc update` writes a new lock");
    }
    let directory = directory.unwrap_or_else(|| PathBuf::from("."));
    match check_lock(&directory) {
        Ok(()) => {
            println!("yashy.lock is up to date");
            exit_code(EXIT_OK)
        }
        Err(error) => {
            error.report();
            error.status()
        }
    }
}

/// Compare `dir/yashy.toml` with `dir/yashy.lock`.
fn check_lock(directory: &Path) -> Result<(), LockCheckError> {
    let manifest_path = directory.join("yashy.toml");
    if !manifest_path.is_file() {
        return Err(LockCheckError::Usage(format!(
            "no yashy.toml in {}",
            directory.display()
        )));
    }
    let manifest = Manifest::load(&manifest_path).map_err(|error| LockCheckError::Diagnostic {
        code: codes::MANIFEST_SYNTAX,
        message: format!(
            "yashy.toml:{}:{}: {}",
            error.line, error.column, error.message
        ),
    })?;
    let lock_path = yashy_driver::workspace::Workspace::shared_file(directory, "yashy.lock");
    if !lock_path.is_file() {
        return Err(LockCheckError::Diagnostic {
            code: codes::LOCKFILE_MISMATCH,
            message: "there is no yashy.lock; run `yashyc update`".to_string(),
        });
    }
    let lock = Lockfile::load(&lock_path).map_err(|error| LockCheckError::Diagnostic {
        code: codes::LOCKFILE_MISMATCH,
        message: format!(
            "yashy.lock:{}:{}: {}",
            error.line, error.column, error.message
        ),
    })?;
    lock.check_manifest(&manifest, directory)
        .map_err(|error| LockCheckError::Diagnostic {
            code: codes::LOCKFILE_MISMATCH,
            message: error.message,
        })?;
    // The stdlib is a build input, so a lock that names it must name the one
    // this compiler will read.
    if yashy_driver::default_stdlib_root().is_some() {
        let version = yashy_driver::stdlib_version();
        let hash = yashy_driver::stdlib_fingerprint();
        lock.check_stdlib(&version, &hash)
            .map_err(|error| LockCheckError::Diagnostic {
                code: codes::LOCKFILE_MISMATCH,
                message: error.message,
            })?;
    }
    Ok(())
}

/// The build-time half of `yashyc lock --check`.
///
/// Returns `Some(exit code)` only when a manifest and a lock both exist and
/// disagree. A manifest without a lock is left alone; the first lock is
/// written by `yashyc update`.
fn verify_project_lock(input: &Path) -> Option<ExitCode> {
    let mut directory = input.parent().unwrap_or_else(|| Path::new("."));
    loop {
        if directory.join("yashy.toml").is_file() {
            break;
        }
        let Some(parent) = directory.parent() else {
            return None;
        };
        directory = parent;
    }
    if !yashy_driver::workspace::Workspace::shared_file(directory, "yashy.lock").is_file() {
        return None;
    }
    match check_lock(directory) {
        Ok(()) => None,
        Err(error) => {
            error.report();
            Some(error.status())
        }
    }
}

fn usage_error(message: &str) -> ExitCode {
    eprintln!("error: {message}\n{USAGE}");
    exit_code(EXIT_USAGE)
}

/// A command-line diagnostic with a code but no source to underline.
///
/// The subject is a flag, not a byte in a file, so there is no span to point
/// at; the code and its `docs:` line are still what a reader needs, because
/// `yashyc explain <code>` is the offline page for the thing that went wrong.
fn coded_error(code: &'static str, message: &str, notes: &[String]) -> ExitCode {
    eprintln!("error[{code}]: {message}");
    for note in notes {
        eprintln!("  {note}");
    }
    eprintln!("  docs: {}", codes::docs_url(code));
    exit_code(EXIT_COMPILE_ERROR)
}

/// The triple `--target <value>` names, or the coded error for one this
/// compiler does not emit for.
///
/// `Ok` carries the `&'static str` spelling, so the value that reaches
/// `BuildOptions`, the IR header and `clang --target` is one string, not four
/// copies that could drift.
fn target_flag(value: &str) -> Result<&'static str, ExitCode> {
    match yashy_driver::target::supported(value) {
        Some(triple) => Ok(triple),
        None => Err(coded_error(
            codes::UNSUPPORTED_TARGET,
            &yashy_driver::target::unsupported_message(value),
            &[],
        )),
    }
}

fn exit_code(code: i32) -> ExitCode {
    ExitCode::from(code as u8)
}

/// `yashyc publish [dir] --registry <root>`: write a package tarball and its
/// index entry.
fn publish_cmd(args: &[String]) -> ExitCode {
    let mut directory: Option<PathBuf> = None;
    let mut registry: Option<String> = None;
    let mut key_id: Option<String> = None;
    let mut iter = args.iter();
    while let Some(argument) = iter.next() {
        match argument.as_str() {
            "--registry" => match iter.next() {
                Some(value) => registry = Some(value.clone()),
                None => return usage_error("`--registry` needs a directory"),
            },
            "--key-id" => match iter.next() {
                Some(value) => key_id = Some(value.clone()),
                None => return usage_error("`--key-id` needs a key name"),
            },
            other if other.starts_with('-') => {
                return usage_error(&format!("unknown option `{other}` for `publish`"))
            }
            other => {
                if directory.is_some() {
                    return usage_error(&format!("unexpected extra argument `{other}`"));
                }
                directory = Some(PathBuf::from(other));
            }
        }
    }
    let Some(registry) = registry else {
        return usage_error("`publish` needs `--registry <dir>`");
    };
    let root = match yashy_driver::registry::registry_path(&registry) {
        Ok(path) => path,
        Err(error) => {
            eprintln!("error: {error}");
            return exit_code(EXIT_COMPILE_ERROR);
        }
    };
    let directory = directory.unwrap_or_else(|| PathBuf::from("."));
    // A package that cannot be reproduced is not publishable: build its
    // program twice in two copies of the tree and compare the bytes. A
    // library with no program has nothing to link, so the gate has nothing to
    // compare and the source-tree hash remains the artifact's identity.
    if let Err(error) = reproducible_publish_gate(&directory) {
        eprintln!("error: not publishable: {error}");
        return exit_code(EXIT_COMPILE_ERROR);
    }
    if let Some(key_id) = key_id {
        let seed_hex = match env::var("YASHY_SIGNING_SEED") {
            Ok(text) => text,
            Err(_) => {
                return usage_error(
                    "`publish --key-id` needs `YASHY_SIGNING_SEED` (hex 32-byte seed)",
                )
            }
        };
        let seed: Vec<u8> = match seed_hex
            .as_bytes()
            .chunks(2)
            .map(|pair| {
                std::str::from_utf8(pair)
                    .ok()
                    .and_then(|text| u8::from_str_radix(text, 16).ok())
            })
            .collect()
        {
            Some(bytes) => bytes,
            None => return usage_error("`YASHY_SIGNING_SEED` must be 64 hexadecimal digits"),
        };
        return match yashy_driver::registry::publish_signed(&directory, &root, &key_id, &seed) {
            Ok((entry, files)) => {
                println!("published {}@{} {}", entry.name, entry.version, entry.hash);
                for file in files {
                    println!("  {}", file.display());
                }
                exit_code(EXIT_OK)
            }
            Err(error) => {
                eprintln!("error: {error}");
                exit_code(EXIT_COMPILE_ERROR)
            }
        };
    }
    match yashy_driver::registry::publish(&directory, &root) {
        Ok(entry) => {
            println!(
                "published {}@{} {} to {}",
                entry.name,
                entry.version,
                entry.hash,
                root.display()
            );
            exit_code(EXIT_OK)
        }
        Err(error) => {
            eprintln!("error: {error}");
            exit_code(EXIT_COMPILE_ERROR)
        }
    }
}

/// Build the package's program twice, in two copies of its tree, and require
/// identical bytes.
///
/// The copies give the build two different absolute paths, which is the
/// failure the gate exists to catch. The child compiler is this same
/// executable, run with `--reproducible` and its working directory inside the
/// copy, so the path-remap flag covers every source the build reads. A missing
/// toolchain or a program that does not build is a refusal, not a skip: a
/// package that cannot be built cannot be reproduced.
fn reproducible_publish_gate(package_root: &Path) -> Result<(), String> {
    let program = match package_program(package_root) {
        Some(program) => program,
        None => return Ok(()),
    };
    let relative = program
        .strip_prefix(package_root)
        .map_err(|_| format!("{} is outside the package", program.display()))?
        .to_path_buf();
    let exe = std::env::current_exe()
        .map_err(|error| format!("cannot find this compiler's own path: {error}"))?;
    let base = std::env::temp_dir().join(format!("yashy-publish-{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);

    let mut images: Vec<Vec<u8>> = Vec::new();
    for label in ["a", "b"] {
        let copy = base.join(label);
        if let Err(error) = copy_package_tree(package_root, &copy) {
            let _ = fs::remove_dir_all(&base);
            return Err(format!("cannot copy the package tree: {error}"));
        }
        let output = Command::new(&exe)
            .arg("build")
            .arg(&relative)
            .arg("--reproducible")
            .arg("-o")
            .arg("publish-check.bin")
            .current_dir(&copy)
            .output()
            .map_err(|error| {
                let _ = fs::remove_dir_all(&base);
                format!("cannot run the compiler: {error}")
            })?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let _ = fs::remove_dir_all(&base);
            return Err(format!(
                "the program does not build:\n{}",
                stderr.trim_end()
            ));
        }
        match fs::read(copy.join("publish-check.bin")) {
            Ok(bytes) => images.push(bytes),
            Err(error) => {
                let _ = fs::remove_dir_all(&base);
                return Err(format!("cannot read the built program: {error}"));
            }
        }
    }
    let _ = fs::remove_dir_all(&base);
    if images.len() == 2 && images[0] != images[1] {
        let offset = images[0]
            .iter()
            .zip(&images[1])
            .position(|(left, right)| left != right)
            .unwrap_or_else(|| images[0].len().min(images[1].len()));
        return Err(format!(
            "two builds of the same tree differ at byte {offset} ({} vs {} bytes); fix the non-input or omit the program",
            images[0].len(),
            images[1].len()
        ));
    }
    Ok(())
}

/// The program the reproducibility gate builds: `src/main.ya`, then
/// `tests/main.ya`. A package with neither is a library.
fn package_program(root: &Path) -> Option<PathBuf> {
    for candidate in ["src/main.ya", "tests/main.ya"] {
        let path = root.join(candidate);
        if path.is_file() {
            return Some(path);
        }
    }
    None
}

/// Copy a package tree, leaving out `.git` and any previous gate output.
fn copy_package_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let name = entry.file_name();
        let text = name.to_string_lossy();
        if text == ".git" || text == "publish-check.bin" {
            continue;
        }
        let source = entry.path();
        let target = to.join(&name);
        let metadata = entry.metadata()?;
        if metadata.is_dir() {
            copy_package_tree(&source, &target)?;
        } else if metadata.is_file() {
            fs::copy(&source, &target)?;
        }
    }
    Ok(())
}

/// `yashyc audit [dir] [--json]`: a read-only package check.
/// `yashyc sbom [dir] [--format spdx-json]`: an SPDX document from the same
/// facts the package audit reads.

/// `yashyc self-update --archive <file> --signature <file>`
///
/// The new binary is verified before the current one is touched. Verification
/// uses the same explicit trust set as the registry; an unsigned archive is
/// refused. The previous binary is kept as `<exe>.old` for rollback.

/// `yashyc attest <tarball> --key-id <id> [--material <file>]...`

/// `yashyc tree [--depth <n>] [--json] [dir]`
/// `yashyc outdated [--registry <dir>] [--json] [dir]`
fn outdated_cmd(args: &[String]) -> ExitCode {
    let mut json = false;
    let mut registry: Option<PathBuf> = None;
    let mut dir: Option<PathBuf> = None;
    let mut iter = args.iter();
    while let Some(argument) = iter.next() {
        match argument.as_str() {
            "--json" => json = true,
            "--registry" => match iter.next() {
                Some(value) => registry = Some(PathBuf::from(value)),
                None => return usage_error("`--registry` needs a directory"),
            },
            other if other.starts_with('-') => {
                return usage_error(&format!("unknown option `{other}` for `outdated`"))
            }
            other => dir = Some(PathBuf::from(other)),
        }
    }
    let project = dir.unwrap_or_else(|| PathBuf::from("."));
    let root = match registry {
        Some(path) => path,
        None => match yashy_driver::registry::registry_path("") {
            Ok(path) => path,
            Err(error) => {
                eprintln!("error: {error}");
                return exit_code(EXIT_USAGE);
            }
        },
    };
    let lock = match Lockfile::load(&yashy_driver::workspace::Workspace::shared_file(
        &project,
        "yashy.lock",
    )) {
        Ok(lock) => lock,
        Err(error) => {
            eprintln!("error: cannot read yashy.lock: {error}");
            return exit_code(EXIT_USAGE);
        }
    };
    let report = match yashy_driver::pkg_cmd::outdated(&lock, &root) {
        Ok(report) => report,
        Err(message) => {
            eprintln!("error: {message}");
            return exit_code(EXIT_USAGE);
        }
    };
    if json {
        println!("{}", report.render());
    } else if let Json::Object(fields) = &report {
        if let Some((_, Json::Array(rows))) = fields.iter().find(|(name, _)| name == "packages") {
            for row in rows {
                if let Json::Object(values) = row {
                    let get = |key: &str| match values.iter().find(|(name, _)| name == key) {
                        Some((_, Json::String(text))) => text.clone(),
                        _ => String::new(),
                    };
                    println!(
                        "{} {} -> {}{}",
                        get("name"),
                        get("locked"),
                        get("newest"),
                        if get("yanked") == "true" {
                            " (yanked)"
                        } else {
                            ""
                        }
                    );
                }
            }
        }
    }
    exit_code(EXIT_OK)
}

/// `yashyc vendor <dir> --registry <dir>`
fn vendor_cmd(args: &[String]) -> ExitCode {
    let mut registry: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    let mut iter = args.iter();
    while let Some(argument) = iter.next() {
        match argument.as_str() {
            "--registry" => match iter.next() {
                Some(value) => registry = Some(PathBuf::from(value)),
                None => return usage_error("`--registry` needs a directory"),
            },
            other if other.starts_with('-') => {
                return usage_error(&format!("unknown option `{other}` for `vendor`"))
            }
            other => out = Some(PathBuf::from(other)),
        }
    }
    let (Some(registry), Some(out)) = (registry, out) else {
        return usage_error("`vendor` needs a destination and `--registry <dir>`");
    };
    match yashy_driver::pkg_cmd::vendor(&registry, &out) {
        Ok((count, bytes)) => {
            println!(
                "vendored {count} archives ({bytes} bytes) to {}",
                out.display()
            );
            exit_code(EXIT_OK)
        }
        Err(message) => {
            eprintln!("error: {message}");
            exit_code(EXIT_USAGE)
        }
    }
}

/// `yashyc yank <name>@<version> --registry <dir> --reason <text>`
fn yank_cmd(args: &[String]) -> ExitCode {
    use yashy_driver::registry;

    let mut spec: Option<String> = None;
    let mut registry: Option<String> = None;
    let mut reason: Option<String> = None;
    let mut iter = args.iter();
    while let Some(argument) = iter.next() {
        match argument.as_str() {
            "--registry" => match iter.next() {
                Some(value) => registry = Some(value.clone()),
                None => return usage_error("`--registry` needs a directory"),
            },
            "--reason" => match iter.next() {
                Some(value) => reason = Some(value.clone()),
                None => return usage_error("`--reason` needs text"),
            },
            other if other.starts_with('-') => {
                return usage_error(&format!("unknown option `{other}` for `yank`"))
            }
            other => spec = Some(other.to_string()),
        }
    }
    let Some(spec) = spec else {
        return usage_error("`yank` needs `<name>@<version>`");
    };
    let Some((name, version)) = spec.split_once('@') else {
        return usage_error("`yank` needs `<name>@<version>`");
    };
    let Some(registry) = registry else {
        return usage_error("`yank` needs `--registry <dir>`");
    };
    let Some(reason) = reason else {
        return usage_error("`yank` needs a non-empty `--reason`");
    };
    let root = match registry::registry_path(&registry) {
        Ok(path) => path,
        Err(error) => {
            eprintln!("error: {error}");
            return exit_code(EXIT_USAGE);
        }
    };
    match registry::yank(&root, name, version, &reason) {
        Ok(files) => {
            println!("yanked {name}@{version}: {reason}");
            for file in files {
                println!("  {}", file.display());
            }
            exit_code(EXIT_OK)
        }
        Err(error) => {
            eprintln!("error: {error}");
            exit_code(EXIT_USAGE)
        }
    }
}

fn tree_cmd(args: &[String]) -> ExitCode {
    let mut depth = usize::MAX;
    let mut json = false;
    let mut dir: Option<PathBuf> = None;
    let mut iter = args.iter();
    while let Some(argument) = iter.next() {
        match argument.as_str() {
            "--depth" => match iter.next().and_then(|value| value.parse::<usize>().ok()) {
                Some(value) => depth = value,
                None => return usage_error("`--depth` needs a whole number"),
            },
            "--json" => json = true,
            other if other.starts_with('-') => {
                return usage_error(&format!("unknown option `{other}` for `tree`"))
            }
            other => dir = Some(PathBuf::from(other)),
        }
    }
    let project = dir.unwrap_or_else(|| PathBuf::from("."));
    let lock = match Lockfile::load(&yashy_driver::workspace::Workspace::shared_file(
        &project,
        "yashy.lock",
    )) {
        Ok(lock) => lock,
        Err(error) => {
            eprintln!("error: cannot read yashy.lock: {error}");
            return exit_code(EXIT_USAGE);
        }
    };
    let features: std::collections::BTreeSet<String> = lock.features.iter().cloned().collect();
    let report = yashy_driver::pkg_cmd::tree(&lock, depth);
    if json {
        println!("{}", report.render());
    } else {
        for line in yashy_driver::pkg_cmd::tree_lines_with_features(&lock, depth, &features) {
            println!("{line}");
        }
    }
    exit_code(EXIT_OK)
}

/// `yashyc why <package> [--json] [dir]`
fn why_cmd(args: &[String]) -> ExitCode {
    let mut json = false;
    let mut name: Option<String> = None;
    let mut dir: Option<PathBuf> = None;
    for argument in args {
        match argument.as_str() {
            "--json" => json = true,
            other if other.starts_with('-') => {
                return usage_error(&format!("unknown option `{other}` for `why`"))
            }
            other if name.is_none() => name = Some(other.to_string()),
            other => dir = Some(PathBuf::from(other)),
        }
    }
    let Some(name) = name else {
        return usage_error("`why` needs a package name");
    };
    let project = dir.unwrap_or_else(|| PathBuf::from("."));
    let lock = match Lockfile::load(&yashy_driver::workspace::Workspace::shared_file(
        &project,
        "yashy.lock",
    )) {
        Ok(lock) => lock,
        Err(error) => {
            eprintln!("error: cannot read yashy.lock: {error}");
            return exit_code(EXIT_USAGE);
        }
    };
    let paths = yashy_driver::pkg_cmd::why(&lock, &name);
    if json {
        let mut report = Json::object();
        report.set("schema_version", Json::number(1));
        report.set("package", Json::string(&name));
        report.set(
            "paths",
            Json::array(
                paths
                    .iter()
                    .map(|path| Json::array(path.iter().map(|name| Json::string(name)).collect()))
                    .collect(),
            ),
        );
        println!("{}", report.render());
    } else if paths.is_empty() {
        println!("no path to {name}");
    } else {
        for path in paths {
            println!("{}", path.join(" -> "));
        }
    }
    exit_code(EXIT_OK)
}

/// `yashyc info <package> [--json] [dir]`
fn info_cmd(args: &[String]) -> ExitCode {
    let mut json = false;
    let mut name: Option<String> = None;
    let mut dir: Option<PathBuf> = None;
    for argument in args {
        match argument.as_str() {
            "--json" => json = true,
            other if other.starts_with('-') => {
                return usage_error(&format!("unknown option `{other}` for `info`"))
            }
            other if name.is_none() => name = Some(other.to_string()),
            other => dir = Some(PathBuf::from(other)),
        }
    }
    let Some(name) = name else {
        return usage_error("`info` needs a package name");
    };
    let project = dir.unwrap_or_else(|| PathBuf::from("."));
    let lock = match Lockfile::load(&yashy_driver::workspace::Workspace::shared_file(
        &project,
        "yashy.lock",
    )) {
        Ok(lock) => lock,
        Err(error) => {
            eprintln!("error: cannot read yashy.lock: {error}");
            return exit_code(EXIT_USAGE);
        }
    };
    let report = match yashy_driver::pkg_cmd::info(&lock, &name, &project) {
        Ok(report) => report,
        Err(message) => {
            eprintln!("error: {message}");
            return exit_code(EXIT_USAGE);
        }
    };
    if json {
        println!("{}", report.render());
    } else if let (
        Some(Json::String(name)),
        Some(Json::String(version)),
        Some(Json::String(source)),
        Some(Json::String(hash)),
        Some(Json::String(license)),
    ) = (
        report_value(&report, "name"),
        report_value(&report, "version"),
        report_value(&report, "source"),
        report_value(&report, "hash"),
        report_value(&report, "license"),
    ) {
        println!("{name} {version}");
        println!("source: {source}");
        println!("hash:   {hash}");
        println!("license: {license}");
    }
    exit_code(EXIT_OK)
}

fn report_value<'a>(report: &'a Json, key: &str) -> Option<&'a Json> {
    match report {
        Json::Object(fields) => fields
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value),
        _ => None,
    }
}

/// `yashyc remove <package> [dir]` — reversible through `yashy.toml.bak`.
fn remove_cmd(args: &[String]) -> ExitCode {
    let mut name: Option<String> = None;
    let mut dir: Option<PathBuf> = None;
    for argument in args {
        match argument.as_str() {
            other if other.starts_with('-') => {
                return usage_error(&format!("unknown option `{other}` for `remove`"))
            }
            other if name.is_none() => name = Some(other.to_string()),
            other => dir = Some(PathBuf::from(other)),
        }
    }
    let Some(name) = name else {
        return usage_error("`remove` needs a package name");
    };
    let project = dir.unwrap_or_else(|| PathBuf::from("."));
    match yashy_driver::pkg_cmd::remove(&project, &name) {
        Ok(files) => {
            println!("removed {name}; changed:");
            for file in files {
                println!("  {}", file.display());
            }
            exit_code(EXIT_OK)
        }
        Err(message) => {
            eprintln!("error: {message}");
            exit_code(EXIT_USAGE)
        }
    }
}

fn attest_cmd(args: &[String]) -> ExitCode {
    let mut artifact: Option<PathBuf> = None;
    let mut key_id: Option<String> = None;
    let mut materials: Vec<PathBuf> = Vec::new();
    let mut iter = args.iter();
    while let Some(argument) = iter.next() {
        match argument.as_str() {
            "--key-id" => match iter.next() {
                Some(value) => key_id = Some(value.clone()),
                None => return usage_error("`--key-id` needs a key name"),
            },
            "--material" => match iter.next() {
                Some(value) => materials.push(PathBuf::from(value)),
                None => return usage_error("`--material` needs a path"),
            },
            other if other.starts_with('-') => {
                return usage_error(&format!("unknown option `{other}` for `attest`"));
            }
            other => {
                if artifact.is_some() {
                    return usage_error("`attest` takes one artifact");
                }
                artifact = Some(PathBuf::from(other));
            }
        }
    }
    let Some(artifact) = artifact else {
        return usage_error("`attest` needs an artifact path");
    };
    let Some(key_id) = key_id else {
        return usage_error("`attest` needs `--key-id <id>`");
    };
    let seed_hex = match env::var("YASHY_SIGNING_SEED") {
        Ok(text) => text,
        Err(_) => return usage_error("`attest` needs `YASHY_SIGNING_SEED` (hex 32-byte seed)"),
    };
    let seed: Vec<u8> = match seed_hex
        .as_bytes()
        .chunks(2)
        .map(|pair| {
            std::str::from_utf8(pair)
                .ok()
                .and_then(|text| u8::from_str_radix(text, 16).ok())
        })
        .collect()
    {
        Some(bytes) => bytes,
        None => return usage_error("`YASHY_SIGNING_SEED` must be 64 hexadecimal digits"),
    };
    let bytes = match fs::read(&artifact) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("error: cannot read {}: {error}", artifact.display());
            return exit_code(EXIT_USAGE);
        }
    };
    for candidate in ["yashy.toml", "yashy.lock", "main.ya"] {
        let path = PathBuf::from(candidate);
        if path.is_file() && !materials.contains(&path) {
            materials.push(path);
        }
    }
    let mut digests = std::collections::BTreeMap::new();
    for path in materials {
        match fs::read(&path) {
            Ok(bytes) => {
                digests.insert(
                    path.to_string_lossy().into_owned(),
                    yashy_manifest::sha256_hex(&bytes),
                );
            }
            Err(error) => {
                eprintln!("error: cannot read material {}: {error}", path.display());
                return exit_code(EXIT_USAGE);
            }
        }
    }
    let statement = yashy_manifest::for_artifact(
        &artifact.to_string_lossy(),
        &bytes,
        host_target_triple(),
        &format!("yashyc {}", env!("CARGO_PKG_VERSION")),
        digests,
    );
    let attestation = PathBuf::from(format!("{}.attestation.json", artifact.display()));
    let signature = match yashy_manifest::sign(&statement.bytes(), &key_id, &seed) {
        Ok(signature) => signature,
        Err(error) => {
            eprintln!("error: {error}");
            return exit_code(EXIT_USAGE);
        }
    };
    if let Err(error) = fs::write(&attestation, statement.bytes()) {
        eprintln!("error: cannot write {}: {error}", attestation.display());
        return exit_code(EXIT_INTERNAL);
    }
    let sig_path = PathBuf::from(format!("{}.attestation.sig", artifact.display()));
    let mut sig_text = yashy_driver::registry::signature_json(&signature).render();
    sig_text.push('\n');
    if let Err(error) = fs::write(&sig_path, sig_text) {
        eprintln!("error: cannot write {}: {error}", sig_path.display());
        return exit_code(EXIT_INTERNAL);
    }
    println!("attestation: {}", attestation.display());
    println!("signature:   {}", sig_path.display());
    exit_code(EXIT_OK)
}

/// `yashyc verify-attestation <artifact> --attestation <file> --signature <file>`
fn verify_attestation_cmd(args: &[String]) -> ExitCode {
    let mut artifact: Option<PathBuf> = None;
    let mut attestation: Option<PathBuf> = None;
    let mut signature: Option<PathBuf> = None;
    let mut rebuild = false;
    let mut iter = args.iter();
    while let Some(argument) = iter.next() {
        match argument.as_str() {
            "--rebuild" => rebuild = true,
            "--attestation" => match iter.next() {
                Some(value) => attestation = Some(PathBuf::from(value)),
                None => return usage_error("`--attestation` needs a path"),
            },
            "--signature" => match iter.next() {
                Some(value) => signature = Some(PathBuf::from(value)),
                None => return usage_error("`--signature` needs a path"),
            },
            other if other.starts_with('-') => {
                return usage_error(&format!(
                    "unknown option `{other}` for `verify-attestation`"
                ));
            }
            other => artifact = Some(PathBuf::from(other)),
        }
    }
    let (Some(artifact), Some(attestation), Some(signature)) = (artifact, attestation, signature)
    else {
        return usage_error(
            "`verify-attestation` needs an artifact, `--attestation` and `--signature`",
        );
    };
    let bytes = match fs::read(&artifact) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("error: cannot read {}: {error}", artifact.display());
            return exit_code(EXIT_USAGE);
        }
    };
    let statement_value = match fs::read(&attestation) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("error: cannot read {}: {error}", attestation.display());
            return exit_code(EXIT_USAGE);
        }
    };
    let parsed_statement = match yashy_manifest::Statement::from_json_text(&statement_value) {
        Ok(statement) => statement,
        Err(error) => {
            eprintln!("error: {error}");
            return exit_code(EXIT_COMPILE_ERROR);
        }
    };
    let sig_text = match fs::read_to_string(&signature) {
        Ok(text) => text,
        Err(error) => {
            eprintln!("error: cannot read {}: {error}", signature.display());
            return exit_code(EXIT_USAGE);
        }
    };
    let keys = match yashy_driver::registry::load_keyring() {
        Ok(keys) if !keys.is_empty() => keys,
        Ok(_) => {
            eprintln!(
                "error: verification needs a trusted key; set YASHY_TRUSTED_KEY or YASHY_KEYRING"
            );
            return exit_code(EXIT_USAGE);
        }
        Err(error) => {
            eprintln!("error: {error}");
            return exit_code(EXIT_USAGE);
        }
    };
    let parsed = match yashy_driver::registry::signature_from_text(&sig_text) {
        Ok(parsed) => parsed,
        Err(error) => {
            eprintln!("error: {error}");
            return exit_code(EXIT_USAGE);
        }
    };
    if let Err(error) = yashy_driver::registry::verify_tarball(&statement_value, &parsed, &keys) {
        eprintln!("error: attestation signature: {error}");
        return exit_code(EXIT_COMPILE_ERROR);
    }
    let digest = yashy_manifest::sha256_hex(&bytes);
    if !parsed_statement
        .subject
        .iter()
        .any(|(_, value)| value == &digest)
    {
        eprintln!("error: the subject digest {digest} does not name this artifact");
        return exit_code(EXIT_COMPILE_ERROR);
    }
    for (material, expected) in &parsed_statement.predicate.materials {
        let path = PathBuf::from(material);
        match fs::read(&path) {
            Ok(bytes) => {
                let actual = yashy_manifest::sha256_hex(&bytes);
                if &actual != expected {
                    eprintln!("error: material `{material}` was {actual}, expected {expected}");
                    return exit_code(EXIT_COMPILE_ERROR);
                }
            }
            Err(error) => {
                eprintln!("error: cannot check material `{material}`: {error}");
                return exit_code(EXIT_COMPILE_ERROR);
            }
        }
    }
    if rebuild {
        let entry = parsed_statement
            .predicate
            .materials
            .keys()
            .find(|path| path.ends_with(".ya"))
            .cloned()
            .ok_or_else(|| {
                eprintln!("error: the statement names no `.ya` entry to rebuild");
                exit_code(EXIT_USAGE)
            });
        let entry = match entry {
            Ok(entry) => entry,
            Err(code) => return code,
        };
        let work = temp_run_dir();
        if let Err(error) = fs::create_dir_all(&work) {
            eprintln!("error: cannot create the rebuild directory: {error}");
            return exit_code(EXIT_INTERNAL);
        }
        for material in parsed_statement.predicate.materials.keys() {
            let source = PathBuf::from(material);
            let target = work.join(&source);
            if let Some(parent) = target.parent() {
                let _ = fs::create_dir_all(parent);
            }
            if let Err(error) = fs::copy(&source, &target) {
                let _ = fs::remove_dir_all(&work);
                eprintln!("error: cannot copy material `{material}`: {error}");
                return exit_code(EXIT_USAGE);
            }
        }
        let out = work.join(format!("rebuilt{}", std::env::consts::EXE_SUFFIX));
        let args = vec![
            entry.clone(),
            "-o".to_string(),
            out.to_string_lossy().into_owned(),
            "--reproducible".to_string(),
        ];
        let code = crate::build(CommandKind::Build, &args);
        let rebuilt = fs::read(&out)
            .ok()
            .map(|bytes| yashy_manifest::sha256_hex(&bytes));
        let _ = fs::remove_dir_all(&work);
        if code != exit_code(EXIT_OK) {
            eprintln!("error: the rebuild failed");
            return exit_code(EXIT_COMPILE_ERROR);
        }
        if rebuilt.as_deref() != Some(digest.as_str()) {
            eprintln!(
                "error: the rebuilt digest {:?} does not match the subject {digest}",
                rebuilt.as_deref().unwrap_or("none")
            );
            return exit_code(EXIT_COMPILE_ERROR);
        }
        println!("rebuild matched the subject digest {digest}");
    }
    println!(
        "attestation verified for {} ({digest}, {} material(s))",
        artifact.display(),
        parsed_statement.predicate.materials.len()
    );
    exit_code(EXIT_OK)
}

fn self_update_cmd(args: &[String]) -> ExitCode {
    let mut archive: Option<PathBuf> = None;
    let mut signature: Option<PathBuf> = None;
    let mut dry_run = false;
    let mut iter = args.iter();
    while let Some(argument) = iter.next() {
        match argument.as_str() {
            "--archive" => match iter.next() {
                Some(value) => archive = Some(PathBuf::from(value)),
                None => return usage_error("`--archive` needs a path"),
            },
            "--signature" => match iter.next() {
                Some(value) => signature = Some(PathBuf::from(value)),
                None => return usage_error("`--signature` needs a path"),
            },
            "--dry-run" => dry_run = true,
            other if other.starts_with('-') => {
                return usage_error(&format!("unknown option `{other}` for `self-update`"))
            }
            other => return usage_error(&format!("unexpected argument `{other}`")),
        }
    }
    let Some(archive) = archive else {
        return usage_error("`self-update` needs `--archive <file>`; see docs/install.md");
    };
    let Some(signature) = signature else {
        return usage_error("`self-update` needs `--signature <file>`; see docs/install.md");
    };
    let bytes = match fs::read(&archive) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("error: cannot read {}: {error}", archive.display());
            return exit_code(EXIT_USAGE);
        }
    };
    let keys = match yashy_driver::registry::load_keyring() {
        Ok(keys) if !keys.is_empty() => keys,
        Ok(_) => {
            eprintln!(
                "error: self-update needs a trusted key; set YASHY_TRUSTED_KEY or YASHY_KEYRING"
            );
            return exit_code(EXIT_USAGE);
        }
        Err(error) => {
            eprintln!("error: {error}");
            return exit_code(EXIT_USAGE);
        }
    };
    let text = match fs::read_to_string(&signature) {
        Ok(text) => text,
        Err(error) => {
            eprintln!("error: cannot read {}: {error}", signature.display());
            return exit_code(EXIT_USAGE);
        }
    };
    let parsed = match yashy_driver::registry::signature_from_text(&text) {
        Ok(parsed) => parsed,
        Err(error) => {
            eprintln!("error: {error}");
            return exit_code(EXIT_USAGE);
        }
    };
    if let Err(error) = yashy_driver::registry::verify_tarball(&bytes, &parsed, &keys) {
        eprintln!("error: {error}");
        return exit_code(EXIT_COMPILE_ERROR);
    }
    if dry_run {
        println!("archive verified; {} bytes, no files changed", bytes.len());
        return exit_code(EXIT_OK);
    }
    let current = match env::current_exe() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("error: cannot locate the running binary: {error}");
            return exit_code(EXIT_INTERNAL);
        }
    };
    let backup = current.with_extension("old");
    let staged = current.with_extension("new");
    if let Err(error) = fs::write(&staged, &bytes) {
        eprintln!("error: cannot stage the new binary: {error}");
        return exit_code(EXIT_INTERNAL);
    }
    if fs::rename(&current, &backup).is_err() || fs::rename(&staged, &current).is_err() {
        let _ = fs::rename(&backup, &current);
        eprintln!("error: the replacement failed; the previous binary was restored");
        return exit_code(EXIT_INTERNAL);
    }
    println!(
        "updated {}; previous binary at {}",
        current.display(),
        backup.display()
    );
    exit_code(EXIT_OK)
}

fn sbom_cmd(args: &[String]) -> ExitCode {
    let mut directory: Option<PathBuf> = None;
    let mut arguments = args.iter();
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            // The format name is accepted for compatibility, but the only
            // rendering is SPDX JSON. Its value still has to be consumed or
            // it would be read as the directory to inspect.
            "--format" => {
                let _ = arguments.next();
            }
            "--json" => {}
            other if other.starts_with('-') => {
                return usage_error(&format!("unknown option `{other}` for `sbom`"));
            }
            other => {
                if directory.is_some() {
                    return usage_error("`sbom` takes one directory");
                }
                directory = Some(PathBuf::from(other));
            }
        }
    }
    let root = directory.unwrap_or_else(|| PathBuf::from("."));
    let mut packages = Vec::new();
    match yashy_manifest::sbom::from_directory(&root) {
        Ok(package) => packages.push(package),
        Err(message) => {
            eprintln!("error: {message}");
            return exit_code(EXIT_USAGE);
        }
    }
    if let Ok(exe) = env::current_exe() {
        if let Ok(bytes) = fs::read(&exe) {
            packages.push(yashy_manifest::sbom::compiler(
                &bytes,
                env!("CARGO_PKG_VERSION"),
            ));
        }
    }
    // The vendored Unicode tables are pinned third-party data. Their exact
    // bytes and release travel in the SBOM so the pin is auditable.
    packages.push(yashy_manifest::sbom::vendored_data(
        "unicode-character-database",
        yashy_runtime::UNICODE_VERSION,
        yashy_runtime::UNICODE_DATA_HEADER.as_bytes(),
        "Unicode-3.0",
    ));
    // The vendored crypto library is pinned by the exact bytes the driver
    // compiles into each program. The hash is over these four sources in the
    // order below, matching packages/crypto/VENDOR.md.
    let mut crypto_sources = Vec::new();
    for bytes in [
        &include_bytes!("../../../packages/crypto/native/monocypher.c")[..],
        &include_bytes!("../../../packages/crypto/native/monocypher.h")[..],
        &include_bytes!("../../../packages/crypto/native/optional/monocypher-ed25519.c")[..],
        &include_bytes!("../../../packages/crypto/native/optional/monocypher-ed25519.h")[..],
    ] {
        crypto_sources.extend_from_slice(bytes);
    }
    packages.push(yashy_manifest::sbom::vendored_data_from(
        "monocypher",
        "4.0.3",
        &crypto_sources,
        "BSD-2-Clause OR CC0-1.0",
        "Loup Vaillant",
    ));
    // The compression dependency is pinned by its own single upstream file.
    packages.push(yashy_manifest::sbom::vendored_data_from(
        "miniz",
        "1.14",
        include_bytes!("../../../packages/crypto/native/miniz.c"),
        "Unlicense",
        "Rich Geldreich",
    ));
    // SHA3 is pinned by the tiny_sha3 commit and the three compiled files.
    let mut sha3_sources = Vec::new();
    for bytes in [
        &include_bytes!("../../../packages/crypto/native/sha3/sha3.c")[..],
        &include_bytes!("../../../packages/crypto/native/sha3/sha3.h")[..],
        &include_bytes!("../../../packages/crypto/native/sha3/LICENSE")[..],
    ] {
        sha3_sources.extend_from_slice(bytes);
    }
    packages.push(yashy_manifest::sbom::vendored_data_from(
        "tiny_sha3",
        "dcbb3192047c2a721f5f851db591871d428036a9",
        &sha3_sources,
        "MIT",
        "Markku-Juhani O. Saarinen",
    ));
    // Zstandard is pinned by its tag and the three compiled files.
    let mut zstd_sources = Vec::new();
    for bytes in [
        &include_bytes!("../../../packages/crypto/native/zstd/zstd.c")[..],
        &include_bytes!("../../../packages/crypto/native/zstd/zstd.h")[..],
        &include_bytes!("../../../packages/crypto/native/zstd/zstd_errors.h")[..],
    ] {
        zstd_sources.extend_from_slice(bytes);
    }
    packages.push(yashy_manifest::sbom::vendored_data_from(
        "zstd",
        "1.5.7",
        &zstd_sources,
        "BSD-3-Clause",
        "Facebook, Inc.",
    ));
    // The vendored time-zone transitions are pinned third-party data too.
    packages.push(yashy_manifest::sbom::vendored_data(
        "tzdata",
        yashy_runtime::ZONE_VERSION,
        yashy_runtime::ZONE_DATA_HEADER.as_bytes(),
        "License-REF-public-domain",
    ));
    let mut metadata = std::collections::BTreeMap::new();
    metadata.insert(
        "compiler_version".to_string(),
        env!("CARGO_PKG_VERSION").to_string(),
    );
    metadata.insert("target".to_string(), host_target_triple().to_string());
    metadata.insert(
        "unicode_version".to_string(),
        yashy_runtime::UNICODE_VERSION.to_string(),
    );
    metadata.insert(
        "tzdata_version".to_string(),
        yashy_runtime::ZONE_VERSION.to_string(),
    );
    println!(
        "{}",
        yashy_manifest::spdx_json_with_metadata(&packages, &metadata).render()
    );
    exit_code(EXIT_OK)
}

/// `yashyc verify-package <tarball> [--registry <dir>]`.
fn verify_package_cmd(args: &[String]) -> ExitCode {
    let mut registry: Option<PathBuf> = None;
    let mut tarball: Option<PathBuf> = None;
    let mut iter = args.iter();
    while let Some(argument) = iter.next() {
        match argument.as_str() {
            "--registry" => match iter.next() {
                Some(value) => registry = Some(PathBuf::from(value)),
                None => return usage_error("`--registry` needs a directory"),
            },
            other if other.starts_with('-') => {
                return usage_error(&format!("unknown option `{other}` for `verify-package`"));
            }
            other => {
                if tarball.is_some() {
                    return usage_error("`verify-package` takes one tarball");
                }
                tarball = Some(PathBuf::from(other));
            }
        }
    }
    let Some(tarball) = tarball else {
        return usage_error("`verify-package` needs a tarball");
    };
    let bytes = match fs::read(&tarball) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("error: cannot read {}: {error}", tarball.display());
            return exit_code(EXIT_USAGE);
        }
    };
    let digest = yashy_manifest::sha256_hex(&bytes);
    let keys = match yashy_driver::registry::load_keyring() {
        Ok(keys) => keys,
        Err(error) => {
            eprintln!("error: {error}");
            return exit_code(EXIT_USAGE);
        }
    };
    let mut findings: Vec<(String, String)> = Vec::new();

    let sig_path = PathBuf::from(format!("{}.sig", tarball.display()));
    let sig_text = fs::read_to_string(&sig_path).ok();
    if keys.is_empty() {
        findings.push((
            yashy_common::codes::SIGNATURE_INVALID.to_string(),
            "no trusted key is configured".to_string(),
        ));
    } else if let Some(text) = sig_text {
        match yashy_driver::registry::signature_from_text(&text) {
            Ok(signature) => {
                if let Err(error) =
                    yashy_driver::registry::verify_tarball(&bytes, &signature, &keys)
                {
                    findings.push((
                        yashy_common::codes::SIGNATURE_INVALID.to_string(),
                        error.to_string(),
                    ));
                }
            }
            Err(error) => findings.push((
                yashy_common::codes::SIGNATURE_INVALID.to_string(),
                error.to_string(),
            )),
        }
    } else {
        findings.push((
            yashy_common::codes::SIGNATURE_INVALID.to_string(),
            format!("no signature at {}", sig_path.display()),
        ));
    }

    // The registry index is the claim a tarball is checked against. With no
    // registry named there is nothing to compare, so the check is opt-in; when
    // one is named, the tarball's tree hash must be an entry the index
    // published. That is the difference between "this tarball is internally
    // consistent" and "this is the package the registry says it is".
    if let Some(root) = &registry {
        match yashy_driver::registry::read_index(root) {
            Ok(entries) => match tarball_tree_hash(&bytes) {
                Ok(tree_hash) => {
                    if !entries.iter().any(|entry| entry.hash == tree_hash) {
                        findings.push((
                            yashy_common::codes::REGISTRY_INDEX_MISMATCH.to_string(),
                            format!(
                                "no registry entry in {} has hash {tree_hash}",
                                root.display()
                            ),
                        ));
                    }
                }
                Err(error) => findings.push((
                    yashy_common::codes::REGISTRY_INDEX_MISMATCH.to_string(),
                    error,
                )),
            },
            Err(error) => findings.push((
                yashy_common::codes::REGISTRY_INDEX_MISMATCH.to_string(),
                error.to_string(),
            )),
        }
    }

    let sbom_dir = registry
        .map(|dir| dir.join("sbom"))
        .unwrap_or_else(|| tarball.parent().unwrap_or(Path::new(".")).to_path_buf());
    let sbom = fs::read_dir(&sbom_dir)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
        .filter_map(|entry| fs::read_to_string(entry.path()).ok())
        .find(|text| text.contains(&digest));
    if sbom.is_none() {
        findings.push((
            yashy_common::codes::SBOM_MISMATCH.to_string(),
            format!("no SBOM in {} names digest {digest}", sbom_dir.display()),
        ));
    }

    let mut report = Json::object();
    report.set("schema_version", Json::number(1));
    report.set("tool", Json::string("verify-package"));
    report.set("package", Json::string(tarball.to_string_lossy()));
    report.set("digest", Json::string(digest));
    let entries: Vec<Json> = findings
        .iter()
        .map(|(code, detail)| {
            let mut item = Json::object();
            item.set("code", Json::string(code));
            item.set("detail", Json::string(detail));
            item
        })
        .collect();
    report.set("findings", Json::array(entries));
    println!("{}", report.render());
    if findings.is_empty() {
        exit_code(EXIT_OK)
    } else {
        exit_code(EXIT_COMPILE_ERROR)
    }
}

/// The tree hash of a tarball's contents, which is what a registry entry names.
///
/// A registry entry records `hash_tree` of the package directory, not the
/// SHA-256 of the `.tar` bytes: the tree hash ignores file order and tar
/// padding, so it is the same for a package however its archive was written.
/// Recomputing it means unpacking, which is why it only runs when a registry
/// was named.
fn tarball_tree_hash(bytes: &[u8]) -> Result<String, String> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!(
        "yashy-verify-package-{}-{stamp}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir)
        .map_err(|error| format!("cannot create {}: {error}", dir.display()))?;
    let result = yashy_driver::registry::extract_tarball(&dir, bytes)
        .map_err(|error| error.to_string())
        .and_then(|()| yashy_manifest::hash_tree(&dir).map_err(|error| error.to_string()));
    let _ = std::fs::remove_dir_all(&dir);
    result
}

fn audit_cmd(args: &[String]) -> ExitCode {
    let json = args.iter().any(|argument| argument == "--json");
    let capabilities = args.iter().any(|argument| argument == "--capabilities");
    let arguments = args
        .iter()
        .filter(|argument| *argument != "--json" && *argument != "--capabilities")
        .cloned()
        .collect::<Vec<_>>();
    let project = match project_args(&arguments, "audit") {
        Ok(project) => project,
        Err(code) => return code,
    };
    let store = match open_store() {
        Ok(store) => store,
        Err(error) => {
            error.report();
            return error.status();
        }
    };
    match yashy_driver::audit::audit_project(&project.directory, &store) {
        Ok(report) => {
            if json {
                println!("{}", report.to_json());
            } else {
                if capabilities {
                    let rendered = if report.capabilities.is_empty() {
                        "none".to_string()
                    } else {
                        report.capabilities.join(", ")
                    };
                    println!("capabilities: {rendered}");
                }
                if report.ok {
                    println!(
                        "ok: {} package{}, {} bytes",
                        report.packages,
                        if report.packages == 1 { "" } else { "s" },
                        report.source_bytes
                    );
                } else {
                    for finding in &report.findings {
                        eprintln!(
                            "error[{}]: package `{}`: {}",
                            finding.code, finding.package, finding.detail
                        );
                    }
                }
            }
            if report.ok {
                exit_code(EXIT_OK)
            } else {
                exit_code(EXIT_COMPILE_ERROR)
            }
        }
        Err(error) => {
            eprintln!("error: {error}");
            exit_code(EXIT_COMPILE_ERROR)
        }
    }
}

/// `yashyc update [--offline] [dir]`: resolve the manifests and rewrite the
/// lockfile from the result.
fn update_cmd(args: &[String]) -> ExitCode {
    let project = match project_args(args, "update") {
        Ok(project) => project,
        Err(code) => return code,
    };
    let store = match open_store() {
        Ok(store) => store,
        Err(error) => {
            error.report();
            return error.status();
        }
    };
    // `update` deliberately ignores the old lock: it is the command that
    // replaces it. Resolution still refuses a cycle or a missing package.
    //
    // A workspace has one lock, so the resolution is the union of every
    // member's graph. Two members that ask for the same package name but
    // different versions or sources are a conflict: one lock cannot pin both.
    let targets = match yashy_driver::workspace::Workspace::shared_root(&project.directory) {
        Some(root) => match yashy_driver::workspace::load(&root) {
            Ok(Some(workspace)) => workspace
                .members
                .iter()
                .map(|member| root.join(member))
                .collect::<Vec<_>>(),
            _ => vec![project.directory.clone()],
        },
        None => vec![project.directory.clone()],
    };
    let mut packages: Vec<yashy_driver::resolve::ResolvedPackage> = Vec::new();
    for target in &targets {
        if !target.join("yashy.toml").is_file() {
            continue;
        }
        let manifest = match load_manifest(target) {
            Ok(manifest) => manifest,
            Err(error) => {
                error.report();
                return error.status();
            }
        };
        let graph = match yashy_driver::resolve(&manifest, None, target, project.offline, &store) {
            Ok(graph) => graph,
            Err(error) => {
                let error = LockCheckError::Diagnostic {
                    code: error.code,
                    message: error.message,
                };
                error.report();
                return error.status();
            }
        };
        for package in graph.packages {
            match packages.iter().find(|known| known.name == package.name) {
                Some(known)
                    if known.version != package.version || known.source != package.source =>
                {
                    let error = LockCheckError::Diagnostic {
                        code: codes::LOCKFILE_MISMATCH,
                        message: format!(
                            "workspace members resolve `{}` differently: {} {} vs {} {}",
                            package.name,
                            known.version,
                            known.source,
                            package.version,
                            package.source
                        ),
                    };
                    error.report();
                    return error.status();
                }
                Some(_) => {}
                None => packages.push(package),
            }
        }
    }
    packages.sort_by(|left, right| left.name.cmp(&right.name));
    let lock = match (yashy_driver::resolve::ResolvedGraph { packages }).to_lock() {
        Ok(lock) => lock,
        Err(error) => {
            let error = LockCheckError::Diagnostic {
                code: error.code,
                message: error.message,
            };
            error.report();
            return error.status();
        }
    };
    // The stdlib is a package input; record the one this compiler will read so
    // a later build can refuse a silently edited tree.
    let mut lock = lock;
    if yashy_driver::default_stdlib_root().is_some() {
        lock.stdlib = Some(yashy_manifest::LockedStdlib {
            version: yashy_driver::stdlib_version(),
            hash: yashy_driver::stdlib_fingerprint(),
        });
    }
    let path = yashy_driver::workspace::Workspace::shared_file(&project.directory, "yashy.lock");
    if let Err(error) = lock.save(&path) {
        eprintln!("error: cannot write {}: {error}", path.display());
        return exit_code(EXIT_COMPILE_ERROR);
    }
    println!(
        "wrote {} ({} package{})",
        path.display(),
        lock.packages.len(),
        if lock.packages.len() == 1 { "" } else { "s" }
    );
    exit_code(EXIT_OK)
}

/// `yashyc fetch [--offline] [dir]`: populate the store without building.
fn fetch_cmd(args: &[String]) -> ExitCode {
    let project = match project_args(args, "fetch") {
        Ok(project) => project,
        Err(code) => return code,
    };
    let manifest = match load_manifest(&project.directory) {
        Ok(manifest) => manifest,
        Err(error) => {
            error.report();
            return error.status();
        }
    };
    let lock = match load_lock_if_present(&project.directory) {
        Ok(lock) => lock,
        Err(error) => {
            error.report();
            return error.status();
        }
    };
    let store = match open_store() {
        Ok(store) => store,
        Err(error) => {
            error.report();
            return error.status();
        }
    };
    match yashy_driver::resolve(
        &manifest,
        lock.as_ref(),
        &project.directory,
        project.offline,
        &store,
    ) {
        Ok(graph) => {
            println!(
                "fetched {} package{} into {}",
                graph.packages.len(),
                if graph.packages.len() == 1 { "" } else { "s" },
                store.root().display()
            );
            exit_code(EXIT_OK)
        }
        Err(error) => {
            let error = LockCheckError::Diagnostic {
                code: error.code,
                message: error.message,
            };
            error.report();
            error.status()
        }
    }
}

/// `yashyc add <name> --path <dir>`, `yashyc add <name> --git <url> --rev
/// <commit>`, or `yashyc add <name>@<version> --registry <dir>`.
fn add_cmd(args: &[String]) -> ExitCode {
    let mut name: Option<String> = None;
    let mut path: Option<String> = None;
    let mut git: Option<String> = None;
    let mut rev: Option<String> = None;
    let mut registry: Option<String> = None;
    let mut allow: Vec<String> = Vec::new();
    let mut iter = args.iter();
    while let Some(argument) = iter.next() {
        match argument.as_str() {
            "--allow" => match iter.next() {
                Some(value) => match yashy_driver::policy::check_request(value, &[]) {
                    Ok(()) => allow.push(value.clone()),
                    Err(message) => return usage_error(&message),
                },
                None => return usage_error("`--allow` needs a capability name"),
            },
            "--path" => match iter.next() {
                Some(value) => path = Some(value.clone()),
                None => return usage_error("`--path` needs a directory"),
            },
            "--git" => match iter.next() {
                Some(value) => git = Some(value.clone()),
                None => return usage_error("`--git` needs a URL"),
            },
            "--rev" => match iter.next() {
                Some(value) => rev = Some(value.clone()),
                None => return usage_error("`--rev` needs a commit"),
            },
            "--registry" => match iter.next() {
                Some(value) => registry = Some(value.clone()),
                None => return usage_error("`--registry` needs a directory"),
            },
            other if other.starts_with('-') => {
                return usage_error(&format!("unknown option `{other}` for `add`"))
            }
            other => {
                if name.is_some() {
                    return usage_error(&format!("unexpected extra argument `{other}`"));
                }
                name = Some(other.to_string());
            }
        }
    }
    let Some(raw_name) = name else {
        return usage_error("`add` needs a dependency name");
    };
    let (name, registry_version) = match raw_name.split_once('@') {
        Some((name, version)) => {
            if yashy_manifest::VersionReq::parse(version).is_none() {
                return usage_error(&format!(
                    "`{version}` is not a version requirement; write `1.2.3`, `^1.2` or `~1.2.0`"
                ));
            }
            (name.to_string(), Some(version.to_string()))
        }
        None => (raw_name, None),
    };
    if !name
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
    {
        return usage_error(&format!(
            "`{name}` is not a package name; use letters, digits, `_` and `-`"
        ));
    }
    if let Some(registry) = &registry {
        let _ = yashy_driver::registry::registry_path(registry);
        env::set_var("YASHY_REGISTRY", registry);
    }
    let block = match (&path, &git, &rev, registry_version) {
        (Some(path), None, None, _) => format!(
            "\n[dependencies.{name}]\npath = \"{}\"\n",
            toml_escape(path)
        ),
        (None, Some(url), Some(rev), _) => format!(
            "\n[dependencies.{name}]\ngit = \"{}\"\nrev = \"{}\"\n",
            toml_escape(url),
            toml_escape(rev)
        ),
        (None, None, None, Some(version)) => format!(
            "\n[dependencies.{name}]\nversion = \"{}\"\n",
            toml_escape(&version)
        ),
        (None, Some(_), None, _) => {
            return usage_error("a git dependency is pinned to a commit; pass `--rev <commit>`")
        }
        (None, None, Some(_), _) | (Some(_), None, Some(_), _) => {
            return usage_error("`--rev` only means something with `--git <url>`")
        }
        (Some(_), Some(_), _, _) => {
            return usage_error("a dependency has exactly one source: `--path` or `--git`")
        }
        (None, None, None, None) => {
            return usage_error(
                "`add` needs a source: `--path <dir>`, `--git <url> --rev <commit>`, or \
                 `<name>@<version> --registry <dir>`",
            )
        }
        (Some(_), _, _, Some(_)) | (None, Some(_), _, Some(_)) => {
            return usage_error("a dependency has exactly one source")
        }
    };
    if !allow.is_empty() {
        println!("adding with capabilities: {}", allow.join(", "));
    }
    let manifest_path = Path::new("yashy.toml").to_path_buf();
    let manifest = match load_manifest_at(&manifest_path) {
        Ok(manifest) => manifest,
        Err(error) => {
            error.report();
            return error.status();
        }
    };
    if manifest.dependencies.contains_key(&name) || manifest.dev_dependencies.contains_key(&name) {
        return usage_error(&format!("`{name}` is already a dependency"));
    }
    let mut text = match fs::read_to_string(&manifest_path) {
        Ok(text) => text,
        Err(error) => {
            eprintln!("error: cannot read {}: {error}", manifest_path.display());
            return exit_code(EXIT_COMPILE_ERROR);
        }
    };
    if !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(&block);
    if let Err(error) = fs::write(&manifest_path, text) {
        eprintln!("error: cannot write {}: {error}", manifest_path.display());
        return exit_code(EXIT_COMPILE_ERROR);
    }
    updating_after_add()
}

/// Resolve and lock the current project after `add` appended its dependency.
fn updating_after_add() -> ExitCode {
    update_cmd(&[])
}

struct ProjectArgs {
    directory: PathBuf,
    offline: bool,
}

fn project_args(args: &[String], command: &str) -> Result<ProjectArgs, ExitCode> {
    let mut directory: Option<PathBuf> = None;
    let mut offline = false;
    for argument in args {
        match argument.as_str() {
            "--offline" => offline = true,
            other if other.starts_with('-') => {
                return Err(usage_error(&format!(
                    "unknown option `{other}` for `{command}`"
                )))
            }
            other => {
                if directory.is_some() {
                    return Err(usage_error(&format!("unexpected extra argument `{other}`")));
                }
                directory = Some(PathBuf::from(other));
            }
        }
    }
    Ok(ProjectArgs {
        directory: directory.unwrap_or_else(|| PathBuf::from(".")),
        offline,
    })
}

fn open_store() -> Result<yashy_driver::Store, LockCheckError> {
    yashy_driver::Store::open()
        .map_err(|error| LockCheckError::Usage(format!("cannot open the package store: {error}")))
}

fn load_manifest(directory: &Path) -> Result<Manifest, LockCheckError> {
    load_manifest_at(&directory.join("yashy.toml"))
}

fn load_manifest_at(path: &Path) -> Result<Manifest, LockCheckError> {
    if !path.is_file() {
        return Err(LockCheckError::Usage(format!(
            "no yashy.toml at {}",
            path.display()
        )));
    }
    Manifest::load(path).map_err(|error| LockCheckError::Diagnostic {
        code: codes::MANIFEST_SYNTAX,
        message: format!(
            "yashy.toml:{}:{}: {}",
            error.line, error.column, error.message
        ),
    })
}

fn load_lock_if_present(directory: &Path) -> Result<Option<Lockfile>, LockCheckError> {
    let path = yashy_driver::workspace::Workspace::shared_file(directory, "yashy.lock");
    if !path.is_file() {
        return Ok(None);
    }
    Lockfile::load(&path)
        .map(Some)
        .map_err(|error| LockCheckError::Diagnostic {
            code: codes::LOCKFILE_MISMATCH,
            message: format!(
                "yashy.lock:{}:{}: {}",
                error.line, error.column, error.message
            ),
        })
}

/// Resolve `input`'s packages and return the module roots to expose.
///
/// A project without a manifest, or a manifest without dependencies, resolves
/// to nothing: the import loader keeps its pre-package behaviour.
fn prepare_packages(input: &Path, offline: bool) -> Result<Vec<PathBuf>, LockCheckError> {
    let mut directory = input.parent().unwrap_or_else(|| Path::new("."));
    loop {
        if directory.join("yashy.toml").is_file() {
            break;
        }
        let Some(parent) = directory.parent() else {
            return Ok(Vec::new());
        };
        directory = parent;
    }
    let manifest_path = directory.join("yashy.toml");
    let manifest = load_manifest_at(&manifest_path)?;
    if manifest.dependencies.is_empty() && manifest.dev_dependencies.is_empty() {
        let src = directory.join("src");
        if src.is_dir() {
            return Ok(vec![src]);
        }
        return Ok(Vec::new());
    }
    let lock = load_lock_if_present(directory)?;
    let store = open_store()?;
    let graph = yashy_driver::resolve(&manifest, lock.as_ref(), directory, offline, &store)
        .map_err(|error| LockCheckError::Diagnostic {
            code: error.code,
            message: error.message,
        })?;
    Ok(graph.module_roots())
}

/// Put the entry directory and every package root at the front of
/// `YASHY_PATH`, keeping whatever the user already had.
fn expose_package_roots(input: &Path, roots: &[PathBuf]) {
    let separator = if cfg!(windows) { ';' } else { ':' };
    let mut all: Vec<PathBuf> = Vec::new();
    let entry_dir = input
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    all.push(entry_dir);
    all.extend(roots.iter().cloned());
    if let Some(existing) = env::var_os("YASHY_PATH") {
        all.extend(
            existing
                .to_string_lossy()
                .split(separator)
                .filter(|root| !root.is_empty())
                .map(PathBuf::from),
        );
    }
    let mut seen = std::collections::BTreeSet::new();
    let text = all
        .into_iter()
        .filter(|path| seen.insert(path.clone()))
        .map(|path| path.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(&separator.to_string());
    env::set_var("YASHY_PATH", text);
}

fn toml_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out
}
